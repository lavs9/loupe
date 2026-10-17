# Loupe

Describe a photo. Matching files come back from this Mac and from any disk you plug in that already holds an index.

The catalog lives next to the photos, in `.photo_index/`. Unplug the disk and that catalog leaves with it. Plug the same disk into another Mac running Loupe and search continues, with no re-index.

Loupe sits in the macOS menu bar. ⇧⌘Space opens search. ⌘Q parks it. Quit is on the menu-bar icon.

**Apple Silicon.** EmbeddingGemma 2 runs on this machine. Nothing about the photos is sent to a server.

## Status

Private preview. The window, menu bar, Settings, and first-run model download work on this Mac.

| Next | |
|---|---|
| Real external disk | [issue #3](https://github.com/lavs9/loupe/issues/3) |
| Signed Mac installer | [issue #4](https://github.com/lavs9/loupe/issues/4) · [docs/APPLE.md](docs/APPLE.md) |
| Photos.app library | [issue #2](https://github.com/lavs9/loupe/issues/2) |

## Run the window

```
cargo build --features gui
./target/debug/loupe gui
```

First launch downloads llama.cpp and the EmbeddingGemma 2 weights into `~/Library/Application Support/loupe/` (~865 MB). llama-server starts on the first search or index.

| Chord | Action |
|---|---|
| ⇧⌘Space | Open Loupe (changeable in Settings) |
| ⌘, | Settings |
| ⌘F | Focus search |
| ⌘O | Grid or list |
| Escape or ⌘Q | Park to the menu bar |

Add folder and Index live on Settings. Search never creates an index.

## Index a folder or a disk

```
loupe index /Volumes/BackupHDD
loupe search "wet street at night 35mm"
loupe status
```

`loupe index` writes `<root>/.photo_index/`:

- `db/catalog.db` — BLAKE3 identity, path, EXIF, FTS5, sqlite-vec
- `models/` — EmbeddingGemma 2 GGUFs, so the drive carries the model
- `cache/previews/` — 384×384 JPEGs

A file with the same path, size, and mtime is skipped. A move with the same bytes updates the path and does not re-embed.

JPEG, PNG, TIFF, WebP, GIF, HEIC, HEIF, AVIF, and common RAW files are indexed. HEIC keeps the orientation Photos shows. RAW uses the embedded JPEG preview.

## Commands

```
loupe gui                 # menu-bar window
loupe gui --self-check    # print folders and exit
loupe index [path]
loupe search <words>
loupe status
loupe sources
loupe sources add <path>
loupe sources remove <path>
loupe serve               # keep the model warm; prints ready <url>
loupe mount [path]        # NFSv3 Search/<query>/
```

`LOUPE_GEMMA_URL` points later commands at a running `loupe serve`. `PHOTO_ENGINE_*` names still work.

## External disks

macOS mounts a plugged-in HDD or SSD at `/Volumes/<name>`. Loupe lists it when `diskutil` reports `Device Location: External`. Index writes `.photo_index` on that volume. Search reads every connected volume that already has one. Listing a disk does not create an index.

## Build

```
cargo build                         # CLI
cargo build --features gui          # CLI + window
./scripts/e2e.sh
./scripts/e2e-gui.sh
./scripts/package-macos.sh          # dist/Loupe.app (unsigned)
```

## Documents

| | |
|---|---|
| [SPEC.md](SPEC.md) | Product and engine |
| [docs/SHIP.md](docs/SHIP.md) | What the installer contains |
| [docs/APPLE.md](docs/APPLE.md) | Apple Developer, signing, notarization |
| Site | `docs/index.html` — GitHub Pages |

The crate and the command are `loupe`. On-disk catalogs stay `.photo_index/`.
