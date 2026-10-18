//! In-process LiteRT-LM EmbeddingEngine (EmbeddingGemma 2 740M).

use anyhow::{Context, Result};
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;
use std::ptr;

use crate::db::EMBED_DIM;

const INPUT_TEXT: i32 = 0;
const INPUT_IMAGE: i32 = 1;
const INPUT_AUDIO: i32 = 3;

type Engine = c_void;
type Settings = c_void;
type Options = c_void;
type Response = c_void;
type InputData = c_void;

struct Api {
    settings_create: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> *mut Settings,
    settings_delete: unsafe extern "C" fn(*mut Settings),
    settings_set_cache_dir: unsafe extern "C" fn(*mut Settings, *const c_char),
    settings_set_vision_tokens: unsafe extern "C" fn(*mut Settings, c_int),
    engine_create: unsafe extern "C" fn(*const Settings) -> *mut Engine,
    engine_delete: unsafe extern "C" fn(*mut Engine),
    options_create: unsafe extern "C" fn() -> *mut Options,
    options_delete: unsafe extern "C" fn(*mut Options),
    options_set_normalize: unsafe extern "C" fn(*mut Options, bool),
    options_set_output_size: unsafe extern "C" fn(*mut Options, c_int),
    options_set_special: unsafe extern "C" fn(*mut Options, bool),
    input_create: unsafe extern "C" fn(c_int, *const c_void, usize) -> *mut InputData,
    input_delete: unsafe extern "C" fn(*mut InputData),
    compute: unsafe extern "C" fn(*mut Engine, *const *const InputData, usize, *const Options) -> *mut Response,
    response_delete: unsafe extern "C" fn(*mut Response),
    response_size: unsafe extern "C" fn(*const Response) -> usize,
    response_values: unsafe extern "C" fn(*const Response) -> *const f32,
}

impl Api {
    unsafe fn load(lib: &Library) -> Result<Self> {
        unsafe {
            Ok(Self {
                settings_create: *lib.get(b"litert_lm_embedding_engine_settings_create\0")?,
                settings_delete: *lib.get(b"litert_lm_embedding_engine_settings_delete\0")?,
                settings_set_cache_dir: *lib.get(b"litert_lm_embedding_engine_settings_set_cache_dir\0")?,
                settings_set_vision_tokens: *lib.get(b"litert_lm_embedding_engine_settings_set_vision_tokens_per_image\0")?,
                engine_create: *lib.get(b"litert_lm_embedding_engine_create\0")?,
                engine_delete: *lib.get(b"litert_lm_embedding_engine_delete\0")?,
                options_create: *lib.get(b"litert_lm_embedding_options_create\0")?,
                options_delete: *lib.get(b"litert_lm_embedding_options_delete\0")?,
                options_set_normalize: *lib.get(b"litert_lm_embedding_options_set_normalize\0")?,
                options_set_output_size: *lib.get(b"litert_lm_embedding_options_set_output_size\0")?,
                options_set_special: *lib.get(b"litert_lm_embedding_options_set_insert_special_tokens\0")?,
                input_create: *lib.get(b"litert_lm_input_data_create\0")?,
                input_delete: *lib.get(b"litert_lm_input_data_delete\0")?,
                compute: *lib.get(b"litert_lm_embedding_engine_compute_embedding\0")?,
                response_delete: *lib.get(b"litert_lm_embedding_response_delete\0")?,
                response_size: *lib.get(b"litert_lm_embedding_response_get_size\0")?,
                response_values: *lib.get(b"litert_lm_embedding_response_get_values\0")?,
            })
        }
    }
}

pub struct LitertEngine {
    api: Api,
    engine: *mut Engine,
    _lib: Library,
}

unsafe impl Send for LitertEngine {}

pub enum Part<'a> {
    Text(&'a str),
    Image(&'a [u8]),
    Audio(&'a [u8]),
}

impl LitertEngine {
    pub fn load(dylib: &Path, model: &Path, cache_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(cache_dir)?;
        let lib = unsafe { Library::new(dylib) }
            .with_context(|| format!("load {}", dylib.display()))?;
        let api = unsafe { Api::load(&lib) }.context("LiteRT embedding symbols")?;
        let engine = create_engine(&api, model, cache_dir, "gpu")
            .or_else(|err| {
                tracing::warn!("LiteRT GPU failed ({err}); trying CPU");
                create_engine(&api, model, cache_dir, "cpu")
            })?;
        Ok(Self {
            api,
            engine,
            _lib: lib,
        })
    }

    pub fn embed(&self, parts: &[Part<'_>]) -> Result<[f32; EMBED_DIM]> {
        if parts.is_empty() {
            anyhow::bail!("nothing to embed");
        }
        let mut inputs = Vec::with_capacity(parts.len());
        for part in parts {
            let (kind, bytes) = match part {
                Part::Text(text) => (INPUT_TEXT, text.as_bytes()),
                Part::Image(bytes) => (INPUT_IMAGE, *bytes),
                Part::Audio(bytes) => (INPUT_AUDIO, *bytes),
            };
            let input = unsafe {
                (self.api.input_create)(kind, bytes.as_ptr() as *const c_void, bytes.len())
            };
            if input.is_null() {
                for held in &inputs {
                    unsafe { (self.api.input_delete)(*held) };
                }
                anyhow::bail!("LiteRT rejected an input part");
            }
            inputs.push(input);
        }
        let ptrs: Vec<*const InputData> = inputs.iter().map(|p| *p as *const InputData).collect();
        let options = unsafe { (self.api.options_create)() };
        if options.is_null() {
            for held in &inputs {
                unsafe { (self.api.input_delete)(*held) };
            }
            anyhow::bail!("LiteRT embedding options failed");
        }
        unsafe {
            (self.api.options_set_normalize)(options, true);
            (self.api.options_set_output_size)(options, EMBED_DIM as c_int);
            (self.api.options_set_special)(options, true);
        }
        let response = unsafe {
            (self.api.compute)(
                self.engine,
                ptrs.as_ptr(),
                ptrs.len(),
                options,
            )
        };
        unsafe { (self.api.options_delete)(options) };
        for held in &inputs {
            unsafe { (self.api.input_delete)(*held) };
        }
        if response.is_null() {
            anyhow::bail!("LiteRT embedding failed");
        }
        let result = copy_embedding(&self.api, response);
        unsafe { (self.api.response_delete)(response) };
        result
    }
}

impl Drop for LitertEngine {
    fn drop(&mut self) {
        if !self.engine.is_null() {
            unsafe { (self.api.engine_delete)(self.engine) };
            self.engine = ptr::null_mut();
        }
    }
}

fn create_engine(api: &Api, model: &Path, cache_dir: &Path, backend: &str) -> Result<*mut Engine> {
    let model = CString::new(model.to_string_lossy().as_bytes())?;
    let backend_c = CString::new(backend)?;
    let cache = CString::new(cache_dir.to_string_lossy().as_bytes())?;
    let settings = unsafe {
        (api.settings_create)(
            model.as_ptr(),
            backend_c.as_ptr(),
            backend_c.as_ptr(),
            backend_c.as_ptr(),
        )
    };
    if settings.is_null() {
        anyhow::bail!("LiteRT could not open EmbeddingGemma 2 ({backend})");
    }
    unsafe {
        (api.settings_set_cache_dir)(settings, cache.as_ptr());
        (api.settings_set_vision_tokens)(settings, 140);
    }
    let engine = unsafe { (api.engine_create)(settings) };
    unsafe { (api.settings_delete)(settings) };
    if engine.is_null() {
        anyhow::bail!("LiteRT could not start EmbeddingGemma 2 ({backend})");
    }
    Ok(engine)
}

fn copy_embedding(api: &Api, response: *mut Response) -> Result<[f32; EMBED_DIM]> {
    let n = unsafe { (api.response_size)(response) };
    let ptr = unsafe { (api.response_values)(response) };
    if ptr.is_null() || n < EMBED_DIM {
        anyhow::bail!("LiteRT returned {n} values, need at least {EMBED_DIM}");
    }
    let full = unsafe { std::slice::from_raw_parts(ptr, n) };
    let mut out = [0f32; EMBED_DIM];
    out.copy_from_slice(&full[..EMBED_DIM]);
    if out.iter().any(|value| !value.is_finite()) {
        anyhow::bail!("LiteRT returned a non-finite value");
    }
    let norm = out.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm < 1e-6 {
        anyhow::bail!("LiteRT returned an empty vector");
    }
    for value in &mut out {
        *value /= norm;
    }
    Ok(out)
}
