# Changelog

## [Unreleased]

## [0.3.1] - 2026-10-26

### Added
- Media Facets / Filter Chips: Segmented control in the toolbar with dynamic counts and icons for All, Photos, Videos, Audio, and Documents.
- Date Range Filter: Filter search results by Past 24 Hours, Past 7 Days, Past 30 Days, This Year, Past Year, or Older than 1 Year via native macOS PopUp button.
- Camera Model Filter: Automatically detects camera models in query results and lets users filter by specific cameras.
- Interactive Scrubber: Drag or click anywhere on the media scrubber track to smoothly seek with a native Apple-style thumb knob.
- Dedicated Volume Controls: In-app player now includes volume slider, mute toggle button (`Volume2`, `Volume1`, `VolumeX`), and percentage readout.
- Track Navigation: Previous / Next track shortcuts (`⌘→` / `⌘←`, `J` / `K`) and Skip Back / Skip Forward buttons on the player card.

### Fixed
- Fixed GitHub Actions CI checkout failure by linking `gpui-apple-hig` to public git repo.
- Document MIME type recognition expanded to include `.pdf`, `.epub`, `.txt`, `.md`, `.rtf`, `.doc`, `.docx`.

## [0.3.0] - 2026-10-24

### Added
- In-app media player for audio (MP3, M4A, FLAC, WAV, AAC) and video (MP4, MOV) playback directly in the preview pane with scrubber, time display, -15s/+15s seeking, and Spacebar play/pause shortcut.
- Interactive video preview stage with click-to-play, centered play overlay, live "PLAYING" badge, and "Open Video" quick-launch button.
- Native Apple Human Interface Guidelines redesign powered by `gpui-apple-hig`: macOS toolbar, search field, pull-down action menus, pop-up sort buttons, and path navigation breadcrumbs.
- Native macOS Appearance Modes: System-based automatic switching, macOS Light Mode, and Apple HIG Dark Graphite mode (replacing pitch black).
- In-memory async metadata, duration, and audio tag caching for stutter-free 60–120 fps rendering.

### Fixed
- Video playback stuck on pause due to premature ticker cancellation during AVPlayer state transition.
- Missing video and unindexed media duration by querying container headers directly via AVURLAsset.
- Main-thread UI stutter caused by synchronous process execution and disk I/O in grid and list render loops.
- Search input latency reduced from 250ms thread-blocking sleep to non-blocking 150ms async debounce.
- Non-standard image formats (grayscale, CMYK, exotic TIFFs) now transcode cleanly via `sips` before LiteRT embedding instead of failing with decode errors.

## [0.2.1] - 2026-10-18

### Fixed
- Songs longer than ~41 seconds were skipped. The embedder only takes 1024 tokens (about 30 seconds of audio). Loupe now sends the start of the track.

## [0.2.0] - 2026-10-18

### Added
- Audio (mp3, m4a, and the usual sound files) and video (mp4, mov, mkv, and the usual containers). One vector per file. A clip is a frame plus the soundtrack.
- EmbeddingGemma 2 runs in-process through LiteRT-LM. First launch downloads about 530 MB.

### Changed
- Site stills replaced with unmarked originals.
- Public site: photos, clips, and songs; first-launch model about 530 MB.
- A catalog built on the previous model is rebuilt, then the old index is deleted.

### Removed
- llama.cpp sidecar and GGUF weights after a folder is on the new model.

## [0.1.2] - 2026-10-18

### Added
- Settings: Watch folders, plus a sweep (Off / 1 hour / 6 hours / Daily). New photos are indexed while Loupe sits in the menu bar.
- In-app update check defaults to `https://lavs9.github.io/loupe/update.json` (GitHub releases).

### Fixed
- Searching a folder that is still being indexed no longer shows `no such table: vec_media`. The other folders still answer.

## [0.1.1] - 2026-10-18

### Fixed
- Settings no longer lists a mounted installer `.dmg` as a plugged-in disk. Opening the image twice had shown Loupe and Loupe 1.

### Changed
- Installer volume is named Install Loupe.
- Site Download is `https://github.com/lavs9/loupe/releases/latest/download/Loupe.dmg`, so it follows the newest release.

## [0.1.0] - 2026-10-17

First public drop. Unsigned Apple Silicon installer.

### Added
- Menu-bar search window. ⇧⌘Space opens it. ⌘Q parks it. Quit is on the tray icon.
- Settings: folders on this Mac, plugged-in disks, Add folder, Choose folder, first-run model download.
- Drive-local `.photo_index` catalogs. Search reads every connected index.
- Unsigned `Loupe-0.1.0.pkg` from `./scripts/package-macos.sh`.

### Notes
- Unplugged disks are skipped. Search keeps running on folders that are still here.
- Gatekeeper: right-click the pkg, choose Open. A signed notarized installer is later ([issue #4](https://github.com/lavs9/loupe/issues/4)).
- Photos.app libraries are later ([issue #2](https://github.com/lavs9/loupe/issues/2)).
