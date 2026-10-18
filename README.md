<p align="center">
  <img src="docs/assets/mark.svg" width="72" height="72" alt="Loupe">
</p>

<h1 align="center">Loupe</h1>

<p align="center"><strong>Describe a photo. Find it on this Mac, and on the disk you plug in.</strong></p>

<p align="center">
  Photo search that lives on the drive.<br>
  The catalog sits next to the pictures, so an HDD carries search with it.
</p>

<p align="center">
  <a href="https://github.com/lavs9/loupe/releases/latest"><img src="https://img.shields.io/github/v/release/lavs9/loupe?label=Loupe&color=1c1814" alt="Release"></a>
  <img src="https://img.shields.io/badge/macOS-14%2B%20Apple%20Silicon-1c1814" alt="Apple Silicon">
  <img src="https://img.shields.io/badge/on--device-EmbeddingGemma%202-1c1814" alt="On device">
  <img src="https://img.shields.io/badge/cloud-never-1c1814" alt="No cloud">
</p>

<p align="center">
  <img src="docs/assets/hero.jpg" alt="A brass loupe on a contact sheet of prints" width="920">
</p>

<p align="center">
  <a href="https://lavs9.github.io/loupe/">Website</a>
  ·
  <a href="https://github.com/lavs9/loupe/releases/download/v0.1.0/Loupe-0.1.0.dmg">Download</a>
  ·
  <a href="SPEC.md">Spec</a>
</p>

---

## What it is

Loupe is a menu-bar app for macOS. You type what you remember about a picture. Matching files come back from:

- any folder you add on this Mac
- a folder on a plugged-in HDD or SSD
- a whole volume you choose

Photos.app searches the Photos library. Spotlight searches names. Loupe searches **what’s in the frame**.

⇧⌘Space opens it. ⌘Q parks it. Quit is on the menu-bar icon.

## The solution

The index is not trapped on one computer.

Loupe writes the catalog **on the disk, next to the photos**, in a hidden `.photo_index/` folder. Unplug the drive and that catalog leaves with the pictures. Plug the same disk into another Mac running Loupe and search continues.

Install Loupe. Connect the disk. No re-index. No copy off the drive.

The model is local. Search is local. Nothing about your photos leaves this Mac or that disk. Ever.

## Numbers

| | |
|---|---|
| Download | ~14 MB unsigned `.dmg` — drag Loupe to Applications |
| On-device model | EmbeddingGemma 2 Q8, ~865 MB, once |
| Memory | sized for **16 GB** unified; 24 and 32 GB run the same model so disks stay portable |
| Catalog | `.photo_index` on the folder or volume you chose |
| Formats | JPEG, HEIC, PNG, TIFF, WebP, GIF, AVIF, RAW |
| Network | photos, queries, and the model stay here |

## Install

Apple Silicon. Unsigned disk image until [issue #4](https://github.com/lavs9/loupe/issues/4).

1. Download [Loupe-0.1.0.dmg](https://github.com/lavs9/loupe/releases/download/v0.1.0/Loupe-0.1.0.dmg).
2. Open it. Drag **Loupe** onto **Applications**.
3. Right-click Loupe, choose **Open**. It sits in the menu bar. ⇧⌘Space opens search.
4. First launch downloads the model into `~/Library/Application Support/loupe/` (~865 MB). If you index a disk, a copy of the model is written onto that disk too.

```
./scripts/package-macos.sh          # dist/Loupe.app + dist/Loupe-0.1.0.dmg
open dist/Loupe-0.1.0.dmg
```

| Chord | |
|---|---|
| ⇧⌘Space | Open |
| ⌘, | Settings |
| ⌘F | Focus search |
| ⌘O | Grid or list |
| Esc or ⌘Q | Park |

Add folder and Index live on Settings. Search never creates an index.

## Index a folder or a disk

```
loupe index /Volumes/BackupHDD
loupe search "wet street at night 35mm"
loupe status
```

`loupe index` writes `<root>/.photo_index/`:

- `db/catalog.db` — BLAKE3 identity, path, EXIF, FTS5, sqlite-vec
- `models/` — EmbeddingGemma 2, so the drive carries the model
- `cache/previews/` — 384×384 JPEGs

A file with the same path, size, and mtime is skipped. A move with the same bytes updates the path and does not re-embed.

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

## Build

```
cargo build                         # CLI
cargo build --features gui          # CLI + window
./scripts/e2e.sh
./scripts/e2e-gui.sh
./scripts/package-macos.sh          # dist/Loupe.app + dist/Loupe-0.1.0.dmg
```

## Documents

| | |
|---|---|
| [SPEC.md](SPEC.md) | Product and engine |
| [docs/SHIP.md](docs/SHIP.md) | What the installer contains |
| [docs/APPLE.md](docs/APPLE.md) | Signing and notarization |
| [MODELS.md](MODELS.md) | RAM picks (not on the public site) |
| Site | https://lavs9.github.io/loupe/ |

The crate and the command are `loupe`. On-disk catalogs stay `.photo_index/`.

Next: [real HDD](https://github.com/lavs9/loupe/issues/3) · [signed installer](https://github.com/lavs9/loupe/issues/4) · [Photos.app](https://github.com/lavs9/loupe/issues/2)
