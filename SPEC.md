# Loupe

Loupe is the product. The crate and the command are `loupe`.

A photographer holds a loupe to a contact sheet. You describe a photo. Matching files come back from this Mac and from any disk you plug in that already holds an index. The catalog lives next to the photos, in `.photo_index/`, so another machine that runs Loupe searches the same disk without copying anything off it.

# Technical Specification: Portable Drive-Centric Multimodal Photo Index & Native OS Search Bridge

* **Target Platforms:** macOS (Apple Silicon / Metal / MLX) primary; Linux & Windows cross-compatible
* **Core Stack:** Rust, LiteRT-LM running EmbeddingGemma 2, SQLite + sqlite-vec, BLAKE3

## 1. Executive Summary
This project builds a storage-centric, zero-reindex multimodal search engine for external photo/media drives. Instead of binding photo indexes and AI model versions to a specific host computer's local application sandbox, the entire intelligence layer—pinned multimodal embedding weights (`EmbeddingGemma 2`), vector indexes, EXIF relational tables, and pre-rendered thumbnails—lives inside a self-contained `.photo_index/` directory directly on the external storage drive alongside the raw media.

## 2. Core Architectural Pillars
* **Pillar 1 — Co-Located Model & Index (`/.photo_index/`):** Both the quantized multimodal model binary and the single-file embedded vector/EXIF database reside on the external drive root.
* **Pillar 2 — Content-Addressable Identity (BLAKE3):** Every media asset is uniquely identified by a fast cryptographic hash (`BLAKE3`) of its content, decoupled from its relative directory path. Moving or renaming folders updates path pointers in milliseconds without re-running vision inference.
* **Pillar 3 — Natively Multimodal Matryoshka Embeddings:** Utilizing `EmbeddingGemma 2` (~740M parameters, ~567 MB quantized), images, text queries, and video frames map directly into a single unified vector space. Using Matryoshka Representation Learning (MRL), 768-d vectors are truncated to 256 dimensions, keeping 100,000 photo embeddings under 50 MB on disk.
* **Pillar 4 — Live Native OS Virtual FS Bridge (NFSv3/v4 Loopback):** Mounts a virtual directory `/Volumes/<Drive>/Search/` where typing any natural language folder name (e.g., `/Search/golden hour mountain portrait 35mm f1.8/`) dynamically resolves in <25ms to zero-copy symlinks of matching photos.

## 3. Our engine

The interaction is Loupe's own. You point it at a folder on this Mac, or at a disk that is plugged in. Loupe writes `.photo_index` there. You ask in words. The files come back. Unplug the disk and that catalog leaves with it. Plug it into another Mac running Loupe and the same catalog is searched, with no re-index. Search never creates an index. Indexing, embeddings, the catalog, and search are this crate.

The model is EmbeddingGemma 2 (Apache 2.0). This crate runs the LiteRT-LM 740M omnimodal bundle (`embeddinggemma-2-740m.litertlm`) through Google AI Edge LiteRT-LM v0.18.0. A photo, a soundtrack, a clip, and the words in a query land in one space. The model emits 768 values. Matryoshka truncation keeps the first 256, and those are normalized again before they are stored. Text queries are prefixed with `task: search query | text:`. Images and audio are sent as native parts. The weights are copied onto the disk at `.photo_index/models/` so the drive carries the model. A second Mac downloads the LiteRT dylib for itself and reads the weights from the disk. A catalog whose `model_version` does not match is skipped by search. Index builds a new catalog beside it, then deletes the old one after that rebuild succeeds. After that, llama.cpp GGUFs in this profile are deleted.

`loupe index <root>` writes `<root>/.photo_index/` (a hidden folder, so the disk stays uncluttered in Finder). On macOS the folder also gets Finder’s hidden flag:

* `config.toml` names the model (`embeddinggemma-2-litert-740m-256`, 256 dimensions).
* `db/catalog.db` holds the BLAKE3 identity, the path, EXIF, an FTS5 keyword index, and a sqlite-vec row.
* `models/` holds `embeddinggemma-2-740m.litertlm`.
* `cache/previews/` holds a JPEG of each decoded frame, fitted inside 384×384. The window also keeps a sharper copy for the contact sheet, read from the photo itself.

A file is skipped when its relative path, size, and mtime are unchanged. A move with the same bytes updates the path and does not re-embed. Paths the scan no longer sees are removed. Replacing the model id builds a new catalog, then deletes the previous one. A photo macOS will not open is left out of that pass. The rest of the folder continues. JPEG, PNG, TIFF, WebP, GIF, HEIC, HEIF, AVIF, and the common RAW extensions are indexed. Video (mp4, mov, m4v, mkv, webm, avi, mpeg, 3gp) is one vector from a representative frame plus the soundtrack. Audio (mp3, m4a, aac, wav, aiff, flac, ogg) is the waveform. PDF and office files wait for a documents release. HEIC and AVIF are converted to JPEG by macOS ImageIO before the vision encoder sees them, so an iPhone photo keeps the orientation it shows in Photos. RAW pixels come from the embedded JPEG preview.

When the chosen folder contains a directory named `Photos`, only that directory is walked. Otherwise the whole folder is walked. Hidden names, `Search`, and `.photo_index` are skipped.

File and folder names, and the camera fields, are matched by FTS5 and merged with the vector rank. Camera terms in the query (`35mm`, `f1.8`, `iso400`, a year) filter before ranking.

`loupe search` reads both sides, and it does not write a catalog:

* **This Mac.** The folder list shows every path remembered in `~/Library/Application Support/loupe/sources.txt`, plus `~/Pictures` when that folder exists. Index or Add folder writes the path into that file. Desktop, Downloads, and the rest of the Mac stay out until you add them. `~/Pictures` is searched only after it has an index. A Photos.app library (a `.photoslibrary` package inside Pictures) is not a source and is not walked; Index on Pictures covers loose files in that folder only. A previous `~/Library/Application Support/photo-engine` folder is renamed to `loupe` the first time Loupe starts.
* **Connected external disks.** macOS mounts a plugged-in HDD or SSD at `/Volumes/<name>`. `diskutil` must report `Device Location: External`. Disk images (`Protocol: Disk Image`, including a mounted installer `.dmg`), the internal boot volume, and Recovery are not listed. `Solid State: Yes` is an SSD, and `No` is a hard disk. A volume whose root contains `.photo_index` is searched. The index stays on that disk, so another machine running this tool sees it when the disk is mounted. Listing a disk does not create an index.

`loupe serve` loads the model once and prints `ready litert`. `LOUPE_GEMMA_URL` / `PHOTO_ENGINE_GEMMA_URL` skip the first-launch download (tests). Without those, each command loads the model and drops it on exit.

`loupe status` prints the photo count for each connected index. `loupe mount` serves `Search/<query>/` over NFSv3. Each result is a relative symlink back to the photo.

The Mac Photos library is a later source. It is not read yet.

## 4. Window

The window is GPUI through the published `gpui-kit` crate, opened with `loupe gui`. The command-line build does not include it. `cargo build --features gui` produces the binary that does. The shell is kit TitleBar and StatusBar. Settings opens from a gear in the title bar. Work height is the viewport minus that chrome. Empty Search and no-results use kit Empty. Hits are a VirtualList. The preview column is a resizable panel. Search also uses Input, ToggleGroup, Carousel, Alert, Skeleton, Progress, DescriptionList, and key chips. This version searches photos, audio, and video. Documents (PDF, Word, Excel, text) are a later version.

`loupe gui --self-check` prints the folder list, the default shortcut, and the update comparison, then exits. It does not open a window and it does not need the `gui` feature.

The window has two screens. Search is the whole window. Settings opens from a gear in the title bar. There is no Search tab.

* **Search.** A query, then the photos from every connected index, with a preview column on the right that follows the selection and can be dragged to resize. The contact sheet is two columns of large prints sized to the sheet width. The preview starts wider than the tiles. An unplugged disk is one short notice, never a list of paths; four missing disks still produce one line. When photos are on screen that line sits in the status bar so the sheet does not move. Empty Search fills the work area with a looping carousel of example queries and the keyboard shortcuts. The contact sheet reads each photo at display size, so an old 32×32 cache is not stretched across the tile. The side preview is a larger copy of the same photo, and it stays visible while you type. Left and right move one photo. Up and down move one row, and the sheet scrolls to keep that photo in view. Arrow keys do this while the query is focused. Command-F focuses the query and selects it, so you can type a new search while looking at photos. Slash does the same while a photo is focused. Return opens the file. Command-Return reveals it in Finder. Command-C copies the file once a photo is focused. Command-O switches between the list and the grid. Command-comma opens Settings. Escape hides the app. The shortcut brings it back. While an index is running, a bar sits above the status bar on Search and Settings, with the current file and how many photos are new or already indexed. Search does not clear that bar.
* **Settings.** Opened from a gear in the title bar, from Command-comma, or from Settings… on the Loupe application menu. Two groups: folders on this Mac, and plugged-in disks. Each row shows the place, how many photos are indexed, and whether search includes it. While that folder is indexing, the row shows an **indexing** tag, a progress bar, and the current file; the Index button stays disabled. Indexing continues if the window is parked. Remembered folders, Pictures when that folder exists, and every mounted volume are listed. A mounted HDD or SSD is listed even when it has no index. **Index** or **Update** runs the same incremental index as `loupe index`, and writes `.photo_index` on that disk. While Loupe is in the menu bar it also watches those folders (FSEvents) and indexes new photos after a two-second quiet period. Writes under `.photo_index` do not retrigger a pass. A sweep every 6 hours (Off / 1 hour / 6 hours / Daily in Settings) catches copies the watcher missed. Unplugged disks are skipped. **Add folder**, on the Settings page next to This Mac, remembers a folder and indexes it. It is not on Search. **Choose folder**, next to Plugged-in disks, opens a picker on that volume so a folder on the disk is indexed instead of the whole disk. The catalog is written in the chosen folder. **Forget** removes a folder from the remembered list and leaves the index on disk. Search never creates an index. A disk that is unplugged is marked not connected and is skipped. Search still runs on every folder that is on this Mac. If macOS blocks the folder itself, the window says so in a sentence. It does not print the raw operating-system error. Settings also shows whether EmbeddingGemma 2 is already on this Mac.

The first launch that has no LiteRT weights and no `LOUPE_GEMMA_URL` downloads LiteRT-LM v0.18.0 and EmbeddingGemma 2 740M into Application Support while the menu-bar process stays parked. A banner reports megabytes. The engine itself starts only on the first search or index. The installer does not ship the ~530 MB weights. See `docs/SHIP.md`.

Loupe sits in the macOS menu bar as an icon, with no title taking extra space. It is an accessory app: no Dock icon. `loupe gui` and Open at login start parked in the background. Closing the window, Escape, or Command-Q parks it; the process stays, the carousel stops, and retained previews are dropped. Click the menu-bar icon or the global shortcut to search. The icon’s menu has Show Loupe, Settings, and Quit Loupe. Quit is on that menu. A second `loupe gui` asks the running copy to show and exits. Settings has **Open at login**, saved as `open_at_login=` in `gui.toml`. That writes `~/Library/LaunchAgents/app.loupe.gui.plist` and talks to launchd.

The global shortcut defaults to ⇧⌘Space (`shift+super+space`). It is stored in `gui.toml` next to `sources.txt`. The chord needs a modifier, then any key macOS can register: a letter, a digit, a function key, an arrow, or punctuation. Changing it replaces the registered shortcut. Closing the window, Escape, or Command-Q parks Loupe, and the shortcut or the menu-bar icon opens it again. Quit Loupe on the menu-bar icon exits.

The window is a darkroom light table. Photos sit on a warm-black surface. Selection is a safelight amber ring. Appearance is Darkroom or Paper, saved as `theme=` in `gui.toml`. The search field placeholder is “Describe a photo”. The wordmark is Loupe. Empty Search shows a looping carousel of example queries and the keyboard shortcuts. The two screens are Search and Settings.

Theme switching uses gpui-kit `ThemeMode` plus Loupe palettes. The `gpui-themes` crate at pierreaubert/gpui-toolkit is a theme editor for a different GPUI stack (`gpui-ui-kit` / `sotf-gpui`) and is not a dependency of this window.

App update is separate from updating an index. The window checks `https://lavs9.github.io/loupe/update.json` (`{"version","url"}`) once a day. The request sends nothing about photos. A newer version offers the GitHub release page. This copy does not download or replace itself. A custom feed URL can be saved in Settings.
