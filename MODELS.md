# Models

Internal notes. Not on the public site. Later embedders are GitHub issues, not landing-page copy.

Loupe searches photos with a **multimodal embedding** model: a photo, a clip, a song, and a sentence land in the same vector space. This is not a chat model. Weights are hundreds of megabytes, not tens of gigabytes. Unified memory on Apple Silicon still matters while indexing, because LiteRT, decode, and macOS share the same pool.

Catalogs are tied to the model id in `.photo_index/config.toml`. A disk indexed with EmbeddingGemma 2 is searched with EmbeddingGemma 2. Mixing models means a re-index of that folder. That is why the **default is one model that fits a 16 GB Mac**, so every HDD stays portable.

## What ships today

**EmbeddingGemma 2 740M LiteRT** (`embeddinggemma-2-litert-740m-256`). Apache 2.0. One `.litertlm` (~485 MB) plus the LiteRT dylib (~138 MB unpacked). Vectors stored at 256 dimensions (Matryoshka from 768). Google AI Edge LiteRT-LM v0.18.0.

Peak unified memory while embedding is about **0.5–2 GB** on top of macOS. A 16 GB Mac has room.

## Pick by RAM

| Unified memory | Recommendation | Why |
|---|---|---|
| **16 GB** | EmbeddingGemma 2 **740M LiteRT** (default) | Fits. Index in the background; close other 4K editors if the fans come on. |
| **24 GB** | EmbeddingGemma 2 **740M LiteRT** | Same model, so disks from a 16 GB Mac still search. Headroom for a 100k library and a browser. |
| **32 GB** | EmbeddingGemma 2 **740M LiteRT** still the portable default | Extra RAM helps decode and batch, not a different catalog. A larger vision embedder is optional later and **re-indexes that disk**. |

Do not put a 2B vision embedder on a 16 GB Mac as the default. It would win some photo benchmarks and lose the “plug this HDD into any Loupe Mac” rule.

## Other models we can offer

Tracked as issues, not site copy: [#5](https://github.com/lavs9/loupe/issues/5) Qwen3-VL-Embedding 2B, [#6](https://github.com/lavs9/loupe/issues/6) jina-embeddings-v5-omni.

These are the next candidates. None of them are wired in the app yet. Each would write a different `model =` in `config.toml`.

| Model | Size | Fits | Strength | Cost to Loupe |
|---|---|---|---|---|
| **EmbeddingGemma 2 Q4** | ~400 MB class | 16 GB, tight | Same space as Q8, slightly softer | Easy: another GGUF pair, same 256-d, same prompts |
| **EmbeddingGemma 2 @ 768-d** | same weights | 24 GB+ | Less truncation, bigger catalogs | Schema: `EMBED_DIM` is 256 today |
| **Qwen3-VL-Embedding 2B** | ~2B, Apache 2.0 | **32 GB** (24 GB possible, tight) | Stronger image / visual-doc scores (MMEB) than Gemma 2 | Needs llama.cpp VL embedding + a new model id; **re-embed the disk** |
| **jina-embeddings-v5-omni** | ~1.7B | **32 GB** | Text, image, video, audio, PDFs — useful when Loupe grows past photos | Different API and dims; later version |
| SigLIP 2 / OpenCLIP | 400M–1B | 16 GB | Image–text only | Gemma 2 already leads it on MIEB lite for this size. Not worth a second catalog. |

Chat models (Gemma 4, Qwen3, Llama) are the wrong tool. They generate words. Loupe needs a vector for every photo.

## Rule we will keep

1. Detect unified memory (`hw.memsize`) and recommend, not auto-switch, unless the user picks a model in Settings.
2. The **portable** choice is always EmbeddingGemma 2 740M LiteRT, so an HDD indexed on a 16 GB machine opens on a 32 GB machine.
3. A “Quality” model on 32 GB is opt-in per folder. That folder’s `.photo_index` stays on that model until they re-index.

## Sources

- [EmbeddingGemma 2](https://blog.google/innovation-and-ai/technology/developers-tools/embeddinggemma-2/) — 740M, ~485 MB LiteRT omnimodal, MRL 768→128, Apache 2.0
- litert-community: `embeddinggemma-2-740m.litertlm` + LiteRT-LM `CLiteRTLM_mac.xcframework` v0.18.0
- Qwen3-VL-Embedding 2B and jina-embeddings-v5-omni as published open weights, 2026
