# Loupe

Loupe is the product. The crate and the command are still `photo-engine`.

A photographer holds a loupe to a contact sheet. You type what the photograph showed. Matching files come back from this Mac and from any disk you plug in that already holds an index. The catalog lives next to the photos, in `.photo_index/`, so another machine that runs Loupe searches the same disk without copying anything off it.

# Technical Specification: Portable Drive-Centric Multimodal Photo Index & Native OS Search Bridge

* **Target Platforms:** macOS (Apple Silicon / Metal / MLX) primary; Linux & Windows cross-compatible
* **Core Stack:** Rust, llama.cpp running EmbeddingGemma 2, SQLite + sqlite-vec, BLAKE3

## 1. Executive Summary
This project builds a storage-centric, zero-reindex multimodal search engine for external photo/media drives. Instead of binding photo indexes and AI model versions to a specific host computer's local application sandbox, the entire intelligence layer—pinned multimodal embedding weights (`EmbeddingGemma 2`), vector indexes, EXIF relational tables, and pre-rendered thumbnails—lives inside a self-contained `.photo_index/` directory directly on the external storage drive alongside the raw media.

## 2. Core Architectural Pillars
* **Pillar 1 — Co-Located Model & Index (`/.photo_index/`):** Both the quantized multimodal model binary and the single-file embedded vector/EXIF database reside on the external drive root.
* **Pillar 2 — Content-Addressable Identity (BLAKE3):** Every media asset is uniquely identified by a fast cryptographic hash (`BLAKE3`) of its content, decoupled from its relative directory path. Moving or renaming folders updates path pointers in milliseconds without re-running vision inference.
* **Pillar 3 — Natively Multimodal Matryoshka Embeddings:** Utilizing `EmbeddingGemma 2` (~740M parameters, ~567 MB quantized), images, text queries, and video frames map directly into a single unified vector space. Using Matryoshka Representation Learning (MRL), 768-d vectors are truncated to 256 dimensions, keeping 100,000 photo embeddings under 50 MB on disk.
* **Pillar 4 — Live Native OS Virtual FS Bridge (NFSv3/v4 Loopback):** Mounts a virtual directory `/Volumes/<Drive>/Search/` where typing any natural language folder name (e.g., `/Search/golden hour mountain portrait 35mm f1.8/`) dynamically resolves in <25ms to zero-copy symlinks of matching photos.

## 3. Our engine

The interaction is Loupe's own. You point it at a folder on this Mac, or at a disk that is plugged in. Loupe writes `.photo_index` there. You ask in words. The files come back. Unplug the disk and that catalog leaves with it. Plug it into another Mac running Loupe and the same catalog is searched, with no re-index. Search never creates an index. Indexing, embeddings, the catalog, and search are this crate.

The model is EmbeddingGemma 2 (Apache 2.0). This crate runs the ggml-org 8-bit text and vision GGUFs through llama.cpp b11642. A photo and the words in a query land in one space. The model emits 768 values. Matryoshka truncation keeps the first 256, and those are normalized again before they are stored. Audio is not loaded. Text queries are prefixed with `task: search result | query:`. Images are sent with no prefix. The weights are copied onto the disk at `.photo_index/models/` so the drive carries the model. A second Mac downloads llama.cpp for itself and reads the weights from the disk.

`photo-engine index <root>` writes `<root>/.photo_index/`:

* `config.toml` names the model (`embeddinggemma-2-q8-256`, 256 dimensions).
* `db/catalog.db` holds the BLAKE3 identity, the path, EXIF, an FTS5 keyword index, and a sqlite-vec row.
* `models/` holds `embeddinggemma-2-Q8_0.gguf` and `mmproj-embeddinggemma-2-Q8_0.gguf`.
* `cache/previews/` holds a JPEG of each decoded frame, fitted inside 384×384. The window also keeps a sharper copy for the contact sheet, read from the photo itself.

A file is skipped when its relative path, size, and mtime are unchanged. A move with the same bytes updates the path and does not re-embed. Paths the scan no longer sees are removed. Replacing the model id re-embeds. A photo macOS will not open is left out of that pass. The rest of the folder continues. JPEG, PNG, TIFF, WebP, GIF, HEIC, HEIF, AVIF, and the common RAW extensions are indexed. HEIC and AVIF are converted to JPEG by macOS ImageIO before the vision encoder sees them, so an iPhone photo keeps the orientation it shows in Photos. RAW pixels come from the embedded JPEG preview.

When the chosen folder contains a directory named `Photos`, only that directory is walked. Otherwise the whole folder is walked. Hidden names, `Search`, and `.photo_index` are skipped.

File and folder names, and the camera fields, are matched by FTS5 and merged with the vector rank. Camera terms in the query (`35mm`, `f1.8`, `iso400`, a year) filter before ranking.

`photo-engine search` reads both sides, and it does not write a catalog:

* **This Mac.** The folder list shows every path remembered in `~/Library/Application Support/photo-engine/sources.txt`, plus `~/Pictures` when that folder exists. Index or Add folder writes the path into that file. Desktop, Downloads, and the rest of the Mac stay out until you add them. `~/Pictures` is searched only after it has an index.
* **Connected external disks.** macOS mounts a plugged-in HDD or SSD at `/Volumes/<name>`. `diskutil` must report `Device Location: External`. The internal boot volume and Recovery are not listed. `Solid State: Yes` is an SSD, and `No` is a hard disk. A volume whose root contains `.photo_index` is searched. The index stays on that disk, so another machine running this tool sees it when the disk is mounted. Listing a disk does not create an index.

`photo-engine serve` loads the model once and prints `ready <url>`. Set `PHOTO_ENGINE_GEMMA_URL` to that URL and later commands reuse it. Without that variable, each command starts the model and stops it on exit.

`photo-engine status` prints the photo count for each connected index. `photo-engine mount` serves `Search/<query>/` over NFSv3. Each result is a relative symlink back to the photo.

The Mac Photos library is a later source. It is not read yet.

## 4. Window

The window is GPUI through the published `gpui-kit` crate, opened with `photo-engine gui`. The command-line build does not include it. `cargo build --features gui` produces the binary that does. Search uses the kit toolbar, key chips, progress bar, and description list, along with its button, input, and checkbox. Version 1 searches images only. Documents, audio, and video are a later version, after image search stays right.

`photo-engine gui --self-check` prints the folder list, the default shortcut, and the update comparison, then exits. It does not open a window and it does not need the `gui` feature.

The window has two screens.

* **Search.** A query, then the photos from every connected index, with a preview column on the right that follows the selection. The contact sheet reads each photo at display size, so an old 32×32 cache is not stretched across the tile. The side preview is a larger copy of the same photo, and it stays visible while you type. Left and right move one photo. Up and down move one row, and the sheet scrolls to keep that photo in view. Arrow keys do this while the query is focused. Return opens the file. Command-Return reveals it in Finder. Command-C copies the file once a photo is focused. Command-O switches between the list and the grid. Escape hides the app. The shortcut brings it back. While an index is running, a bar stays on the search screen and the folder screen, with the current file and how many photos are new or already indexed. Search does not clear that bar.
* **Disks.** Two groups: folders on this Mac, and plugged-in disks. Each row shows the place, how many photos are indexed, and whether search includes it. Remembered folders, Pictures when that folder exists, and every mounted volume are listed. A mounted HDD or SSD is listed even when it has no index. **Index** or **Update** runs the same incremental index as `photo-engine index`, and writes `.photo_index` on that disk. **Add folder** remembers a folder and indexes it. **Forget** removes a folder from the remembered list and leaves the index on disk. Search never creates an index. A disk that is unplugged is marked not connected and is not searched. If macOS blocks the folder itself, the window says so in a sentence. It does not print the raw operating-system error.

The global shortcut defaults to ⇧⌘Space (`shift+super+space`). It is stored in `gui.toml` next to `sources.txt`. The chord needs a modifier, then space, a letter, or f1 through f24. Changing it replaces the registered shortcut. Closing the window hides Loupe, and the shortcut opens it again. Quit, on the Folders screen or with Command-Q, exits.

The window is a darkroom light table. Photos sit on a warm-black surface. Selection is a safelight amber ring. The wordmark is Loupe. The two screens are Search and Disks.

App update is separate from updating an index. The window can check one JSON feed, `{"version","url"}`, once a day. The request sends nothing about photos. An empty feed does not touch the network. A newer version offers the release page. This copy does not download or replace itself. Set the feed in the window, or leave it empty.
