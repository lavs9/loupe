//! macOS PhotoKit bridge for indexing Apple Photos library.
//!
//! Accesses the user's Photos library via PhotoKit (PHAsset / PHImageManager)
//! instead of walking the .photoslibrary package directly (which is TCC-blocked
//! and misses user edits).
//!
//! Stores the catalog under `~/Library/Application Support/loupe/photos-catalog/`.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoAuthStatus {
    NotDetermined = 0,
    Restricted = 1,
    Denied = 2,
    Authorized = 3,
    Limited = 4,
}

impl PhotoAuthStatus {
    pub fn is_authorized(self) -> bool {
        matches!(self, Self::Authorized | Self::Limited)
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NotDetermined => "Permission needed",
            Self::Restricted => "Restricted",
            Self::Denied => "Access denied in System Settings",
            Self::Authorized => "Full Photos access",
            Self::Limited => "Selected Photos access",
        }
    }
}

impl From<i32> for PhotoAuthStatus {
    fn from(val: i32) -> Self {
        match val {
            1 => Self::Restricted,
            2 => Self::Denied,
            3 => Self::Authorized,
            4 => Self::Limited,
            _ => Self::NotDetermined,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PhotoAsset {
    pub id: String,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub width: u32,
    pub height: u32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn photokit_auth_status() -> i32;
    fn photokit_request_auth() -> i32;
    fn photokit_fetch_assets(
        cb: extern "C" fn(
            local_id: *const libc::c_char,
            created: f64,
            modified: f64,
            width: i32,
            height: i32,
            ctx: *mut libc::c_void,
        ),
        ctx: *mut libc::c_void,
    ) -> i32;
    fn photokit_get_asset_image_data(
        local_id: *const libc::c_char,
        max_dim: i32,
        cb: extern "C" fn(bytes: *const u8, len: libc::size_t, ctx: *mut libc::c_void),
        ctx: *mut libc::c_void,
    ) -> i32;
    fn photokit_reveal_in_photos(local_id: *const libc::c_char) -> i32;
}

/// Where Loupe stores the Apple Photos catalog.
/// Always lives in Application Support, never inside Apple's package.
pub fn photos_catalog_dir() -> PathBuf {
    crate::sources::state_dir().join("photos-catalog")
}

/// Check if a directory is the Apple Photos catalog directory.
pub fn is_photos_catalog(path: &Path) -> bool {
    path == photos_catalog_dir()
}

/// Check if a file path is a cached photo from the Photos catalog.
pub fn is_photos_catalog_image(path: &Path) -> bool {
    path.starts_with(photos_catalog_dir().join("images"))
}

/// Convert a PhotoKit local identifier into a filesystem-safe relative path.
pub fn safe_asset_rel_path(local_id: &str) -> PathBuf {
    let safe_name = local_id.replace('/', "__");
    PathBuf::from("images").join(format!("{safe_name}.jpg"))
}

/// Extract the PhotoKit local identifier from an image path inside `photos-catalog/images/`.
pub fn local_id_from_image_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(stem.replace("__", "/"))
}

/// Current PhotoKit authorization status.
pub fn authorization_status() -> PhotoAuthStatus {
    if let Ok(mock) = std::env::var("LOUPE_PHOTOS_AUTH") {
        return match mock.to_ascii_lowercase().as_str() {
            "authorized" => PhotoAuthStatus::Authorized,
            "limited" => PhotoAuthStatus::Limited,
            "denied" => PhotoAuthStatus::Denied,
            "restricted" => PhotoAuthStatus::Restricted,
            _ => PhotoAuthStatus::NotDetermined,
        };
    }
    #[cfg(target_os = "macos")]
    unsafe {
        PhotoAuthStatus::from(photokit_auth_status())
    }
    #[cfg(not(target_os = "macos"))]
    PhotoAuthStatus::NotDetermined
}

/// Request access to the Photos library. Prompts user if status is NotDetermined.
pub fn request_authorization() -> PhotoAuthStatus {
    if let Ok(mock) = std::env::var("LOUPE_PHOTOS_AUTH") {
        return match mock.to_ascii_lowercase().as_str() {
            "authorized" => PhotoAuthStatus::Authorized,
            "limited" => PhotoAuthStatus::Limited,
            "denied" => PhotoAuthStatus::Denied,
            "restricted" => PhotoAuthStatus::Restricted,
            _ => PhotoAuthStatus::NotDetermined,
        };
    }
    #[cfg(target_os = "macos")]
    unsafe {
        PhotoAuthStatus::from(photokit_request_auth())
    }
    #[cfg(not(target_os = "macos"))]
    PhotoAuthStatus::Denied
}

/// Fetch all image assets from the user's Photos library.
pub fn fetch_all_assets() -> Result<Vec<PhotoAsset>> {
    if let Ok(mock_dir) = std::env::var("LOUPE_PHOTOS_MOCK_DIR") {
        let path = PathBuf::from(mock_dir);
        let mut mock_assets = Vec::new();
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                for (idx, entry) in entries.flatten().enumerate() {
                    let p = entry.path();
                    if p.is_file() {
                        let id = format!("MOCK-{idx:04}/L0/001");
                        mock_assets.push(PhotoAsset {
                            id,
                            created_at: Some(1700000000 + idx as i64),
                            modified_at: Some(1700000000 + idx as i64),
                            width: 1920,
                            height: 1080,
                        });
                    }
                }
            }
        }
        return Ok(mock_assets);
    }

    #[cfg(target_os = "macos")]
    {
        extern "C" fn asset_cb(
            local_id: *const libc::c_char,
            created: f64,
            modified: f64,
            width: i32,
            height: i32,
            ctx: *mut libc::c_void,
        ) {
            unsafe {
                let assets = &mut *(ctx as *mut Vec<PhotoAsset>);
                let id = if local_id.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(local_id).to_string_lossy().into_owned()
                };
                assets.push(PhotoAsset {
                    id,
                    created_at: if created > 0.0 { Some(created as i64) } else { None },
                    modified_at: if modified > 0.0 { Some(modified as i64) } else { None },
                    width: width.max(0) as u32,
                    height: height.max(0) as u32,
                });
            }
        }

        let mut assets = Vec::new();
        let ret = unsafe {
            photokit_fetch_assets(
                asset_cb,
                &mut assets as *mut Vec<PhotoAsset> as *mut libc::c_void,
            )
        };
        if ret < 0 {
            bail!("Photos library access not authorized");
        }
        Ok(assets)
    }
    #[cfg(not(target_os = "macos"))]
    bail!("PhotoKit is only supported on macOS")
}

/// Request image JPEG data for a specific asset.
/// Returns `Ok(Some(bytes))` on success, `Ok(None)` if the original is iCloud-only and not local.
pub fn get_asset_image_jpeg(local_id: &str, max_dim: u32) -> Result<Option<Vec<u8>>> {
    if let Ok(mock_dir) = std::env::var("LOUPE_PHOTOS_MOCK_DIR") {
        let path = PathBuf::from(mock_dir);
        // Find corresponding mock file
        if let Ok(entries) = std::fs::read_dir(&path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if let Ok(bytes) = std::fs::read(&p) {
                        return Ok(Some(bytes));
                    }
                }
            }
        }
        return Ok(None);
    }

    #[cfg(target_os = "macos")]
    {
        extern "C" fn data_cb(bytes: *const u8, len: libc::size_t, ctx: *mut libc::c_void) {
            unsafe {
                let out = &mut *(ctx as *mut Vec<u8>);
                if !bytes.is_null() && len > 0 {
                    let slice = std::slice::from_raw_parts(bytes, len);
                    out.extend_from_slice(slice);
                }
            }
        }

        let c_id = std::ffi::CString::new(local_id).context("invalid local_id string")?;
        let mut out = Vec::new();
        let status = unsafe {
            photokit_get_asset_image_data(
                c_id.as_ptr(),
                max_dim as i32,
                data_cb,
                &mut out as *mut Vec<u8> as *mut libc::c_void,
            )
        };

        match status {
            0 => {
                if out.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(out))
                }
            }
            1 => {
                // iCloud only, skipped
                Ok(None)
            }
            _ => Ok(None),
        }
    }
    #[cfg(not(target_os = "macos"))]
    bail!("PhotoKit is only supported on macOS")
}

/// Show / reveal the photo in macOS Photos app.
pub fn reveal_in_photos(local_id: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let c_id = std::ffi::CString::new(local_id).context("invalid local_id string")?;
        let success = unsafe { photokit_reveal_in_photos(c_id.as_ptr()) };
        if success == 1 {
            Ok(())
        } else {
            bail!("Failed to reveal photo {local_id} in Photos app")
        }
    }
    #[cfg(not(target_os = "macos"))]
    bail!("PhotoKit is only supported on macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_status_conversions() {
        assert_eq!(PhotoAuthStatus::from(0), PhotoAuthStatus::NotDetermined);
        assert_eq!(PhotoAuthStatus::from(1), PhotoAuthStatus::Restricted);
        assert_eq!(PhotoAuthStatus::from(2), PhotoAuthStatus::Denied);
        assert_eq!(PhotoAuthStatus::from(3), PhotoAuthStatus::Authorized);
        assert_eq!(PhotoAuthStatus::from(4), PhotoAuthStatus::Limited);

        assert!(PhotoAuthStatus::Authorized.is_authorized());
        assert!(PhotoAuthStatus::Limited.is_authorized());
        assert!(!PhotoAuthStatus::Denied.is_authorized());
        assert!(!PhotoAuthStatus::NotDetermined.is_authorized());
    }

    #[test]
    fn test_safe_asset_paths() {
        let local_id = "9F9247E6-148F-4D78-9B7E-7F89812A27F1/L0/001";
        let rel = safe_asset_rel_path(local_id);
        assert_eq!(
            rel,
            PathBuf::from("images/9F9247E6-148F-4D78-9B7E-7F89812A27F1__L0__001.jpg")
        );

        let roundtrip = local_id_from_image_path(&rel);
        assert_eq!(roundtrip.as_deref(), Some(local_id));
    }

    #[test]
    fn test_photos_catalog_dir_in_app_support() {
        let dir = photos_catalog_dir();
        assert!(dir.ends_with("loupe/photos-catalog") || dir.ends_with("photo-engine/photos-catalog"));
        assert!(is_photos_catalog(&dir));
    }

    struct MockEmbedder;
    impl crate::embedder::MultimodalEmbedder for MockEmbedder {
        fn model_version(&self) -> &str {
            crate::embedder::MODEL_ID
        }
        fn embed_image(&self, _: &Path) -> Result<Option<[f32; crate::db::EMBED_DIM]>> {
            Ok(Some([0.1; crate::db::EMBED_DIM]))
        }
        fn embed_audio(&self, _: &Path) -> Result<Option<[f32; crate::db::EMBED_DIM]>> {
            Ok(None)
        }
        fn embed_multimodal(
            &self,
            _: Option<&[u8]>,
            _: Option<&[u8]>,
        ) -> Result<Option<[f32; crate::db::EMBED_DIM]>> {
            Ok(None)
        }
        fn embed_text(&self, _: &str) -> Result<[f32; crate::db::EMBED_DIM]> {
            Ok([0.1; crate::db::EMBED_DIM])
        }
        fn embed_document(&self, _: &str) -> Result<[f32; crate::db::EMBED_DIM]> {
            Ok([0.1; crate::db::EMBED_DIM])
        }
    }

    #[test]
    fn test_photos_indexing_pipeline_end_to_end() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("loupe-pk-test-{stamp}"));
        let mock_photos_dir = temp_dir.join("mock_photos");
        let mock_state_dir = temp_dir.join("state");
        std::fs::create_dir_all(&mock_photos_dir).unwrap();
        std::fs::create_dir_all(&mock_state_dir).unwrap();

        // Create a test image
        let img_path = mock_photos_dir.join("shot1.jpg");
        image::RgbImage::new(32, 32).save(&img_path).unwrap();

        std::env::set_var("LOUPE_PHOTOS_AUTH", "authorized");
        std::env::set_var("LOUPE_PHOTOS_MOCK_DIR", mock_photos_dir.to_str().unwrap());
        std::env::set_var("LOUPE_STATE_DIR", mock_state_dir.to_str().unwrap());

        let embedder = MockEmbedder;
        let live = std::sync::Arc::new(std::sync::Mutex::new(crate::app_model::IndexLive::default()));

        let outcome = crate::app_model::index_photos_live(&embedder, &live);
        assert!(outcome.is_ok(), "index_photos_live should succeed: {:?}", outcome.err());
        let outcome = outcome.unwrap();
        assert_eq!(outcome.report.embedded, 1);

        let cat_db = mock_state_dir.join("photos-catalog/.photo_index/db/catalog.db");
        assert!(cat_db.is_file(), "catalog database should exist");
        let db = crate::db::CatalogDb::open_readonly(&cat_db).unwrap();
        let stats = db.stats().unwrap();
        assert_eq!(stats.assets, 1);

        // Run incremental index again without changes - should be unchanged
        let outcome_2 = crate::app_model::index_photos_live(&embedder, &live).unwrap();
        assert_eq!(outcome_2.report.unchanged, 1);
        assert_eq!(outcome_2.report.embedded, 0);

        std::env::remove_var("LOUPE_PHOTOS_AUTH");
        std::env::remove_var("LOUPE_PHOTOS_MOCK_DIR");
        std::env::remove_var("LOUPE_STATE_DIR");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
