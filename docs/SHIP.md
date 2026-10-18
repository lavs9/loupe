# Shipping Loupe on macOS

Loupe is an accessory menu-bar app. The public download is an **unsigned** `.dmg`: open it, drag Loupe to Applications, right-click Open. The site Download button is `https://github.com/lavs9/loupe/releases/latest/download/Loupe.dmg` (stable filename on the newest GitHub release). A signed, notarized image is [issue #4](https://github.com/lavs9/loupe/issues/4). The EmbeddingGemma 2 weights (~865 MB) are **not** in the image. First launch downloads them into `~/Library/Application Support/loupe/`.

## What we ship

| Piece | Where it lives | Size |
|---|---|---|
| `Loupe.app` | `/Applications` | the `loupe` binary with `--features gui`, Info.plist, icon |
| llama.cpp b11642 | downloaded into Application Support/runtime | ~12 MB |
| `embeddinggemma-2-Q8_0.gguf` + mmproj | downloaded into Application Support/models | ~865 MB |
| `.photo_index/` | next to the photos, never in the app bundle | catalog |

LSUIElement is true. No Dock icon. Open at login is a Settings checkbox that writes `~/Library/LaunchAgents/app.loupe.gui.plist`.

The full Apple account, certificate, icon, and notarization checklist is [APPLE.md](APPLE.md).

## One-time Apple setup

1. Enroll in the [Apple Developer Program](https://developer.apple.com/programs/) ($99 / year).
2. In Certificates, Identifiers & Profiles, create:
   - **Developer ID Application** (signs the `.app`)
   - **Developer ID Installer** (signs the `.pkg`)
3. Install both into the login keychain.
4. Create an app-specific password for notarization, or an App Store Connect API key (`AuthKey_*.p8`).
5. Apple Team ID, e.g. `ABCD123456`. Bundle id: `app.loupe.gui`.

Without those certificates `./scripts/package-macos.sh` still builds an ad-hoc-signed `.app` and an unsigned `.dmg` (Loupe.app + Applications shortcut). Gatekeeper wants right-click Open on the app after you drag it. Pass `--pkg` only if you need a silent installer into `/Applications`.

## Build and sign

```
# unsigned .app + .dmg
./scripts/package-macos.sh

# signed + notarized .dmg (needs Developer ID Application)
IDENTITY_APP="Developer ID Application: Mayank Lavania (TEAMID)" \
APPLE_ID="you@domain" \
APPLE_TEAM_ID="TEAMID" \
APPLE_APP_PASSWORD="xxxx-xxxx-xxxx-xxxx" \
./scripts/package-macos.sh --sign --notarize
```

The script:

1. `cargo build --release --features gui`
2. Lays out `dist/Loupe.app/Contents/{MacOS,Resources,Info.plist}`
3. `codesign` the app
4. `hdiutil create` a UDZO image (`-volname "Install Loupe"`) with Loupe.app and an Applications shortcut. Settings does not list that mounted image as a plugged-in disk.
5. With `--notarize`, `notarytool submit` the **dmg**, then staple
6. `--pkg` is optional: a silent installer into `/Applications`

Gatekeeper then allows a double-click without a right-click Open.

## Entitlements

`scripts/Loupe.entitlements` is the hardened-runtime set:

- `network.client` — Hugging Face + GitHub for the first-run model, update feed
- `files.user-selected.read-write` — Add folder / Index
- `files.downloads.read-write` is **not** requested; Pictures is opted in by the user

Photos library access is a later version.

## First-run loadup

1. User opens the `.dmg`, drags Loupe to Applications, launches it. The window opens. The process takes the menu-bar seat.
2. If Application Support has no GGUF files and `LOUPE_GEMMA_URL` is unset, first launch is a full screen: downloading the on-device model. Hide to menu bar; come back when it finishes.
3. llama-server is **not** started during the fetch. RSS stays near the idle 46 MB plus download buffers.
4. First search or Index starts llama-server against those files. A later launch reuses `llama-server.url` if that process is still healthy.
5. Indexing a folder copies (or hard-links) the weights into that folder’s `.photo_index/models/` so the disk carries the model.

A LaunchAgent start at login follows the same path. The fetch is skipped when the files are already there.

## Updates

The in-app update feed is a JSON `{version,url}` that opens a release page. It does not replace the binary. Sparkle or a signed delta is a later version, after the first signed pkg is in people’s hands.

## Not this version

- App Store / sandbox. User-selected folders plus a sidecar `.photo_index` on external disks fights the sandbox. Developer ID outside the store is the path.
- Bundling GGUF in the dmg. 865 MB on every update is worse than a first-run download with a progress bar.
- Intel Mac. The pinned llama.cpp build is Apple Silicon.

## Check before a human gets the dmg

- `scripts/e2e-gui.sh` (isolated state, must not download, must not start llama)
- Idle 45 s: footprint stable, leak bytes stable
- First-run on a clean Application Support: banner reaches 100 %, then a search returns photos
- After staple, open the dmg, drag to Applications, double-click without right-click Open
