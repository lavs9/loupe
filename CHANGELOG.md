# Changelog

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
