# Apple release checklist

Goal: a Gatekeeper-clean `.pkg` that installs `Loupe.app` in `/Applications`, signed with Developer ID, notarized, stapled. Not the Mac App Store this version.

The packaging script is `scripts/package-macos.sh`. The entitlements file is `scripts/Loupe.entitlements`. Bundle id: `app.loupe.gui`.

This Mac currently has **0 signing identities**. v0.1.0 ships an **unsigned** `.pkg` (right-click Open). The steps below are what turn that into a Gatekeeper-clean double-click.

## 1. Account

- [ ] Enroll in the [Apple Developer Program](https://developer.apple.com/programs/) ($99 / year). Individual is enough.
- [ ] Sign the latest Paid Applications and Developer agreements in [App Store Connect](https://appstoreconnect.apple.com) / [developer.apple.com](https://developer.apple.com/account).
- [ ] Note **Team ID** (10 characters). Put it in the env for the packager.
- [ ] Confirm the Apple ID email that will own notarization.

## 2. Signing identities

In Certificates, Identifiers & Profiles:

- [ ] Identifier `app.loupe.gui` (App ID, macOS).
- [ ] **Developer ID Application** certificate — signs `Loupe.app`.
- [ ] **Developer ID Installer** certificate — signs the `.pkg`.
- [ ] Download both `.cer` files, double-click into the **login** keychain.
- [ ] `security find-identity -v -p codesigning` shows both. The names look like `Developer ID Application: Mayank Lavania (TEAMID)`.

## 3. Notarization credentials

Pick one:

- [ ] App-specific password for the Apple ID, stored in the keychain as `notarytool` profile, **or**
- [ ] App Store Connect API key (`AuthKey_*.p8` + Issuer ID + Key ID). Keep the `.p8` out of git.

```
xcrun notarytool store-credentials loupe \
  --apple-id "$APPLE_ID" \
  --team-id "$APPLE_TEAM_ID" \
  --password "$APPLE_APP_PASSWORD"
```

## 4. App bundle work still needed

- [ ] **App icon** `.icns` in `Loupe.app/Contents/Resources/AppIcon.icns`. `package-macos.sh` names `AppIcon` in Info.plist and does not copy an icon yet.
- [ ] Info.plist: `CFBundleIconFile`, `NSHighResolutionCapable`, `LSUIElement`, `LSMinimumSystemVersion` 14.0, copyright, `NSPhotoLibraryUsageDescription` when Photos ships.
- [ ] Hardened runtime (`codesign --options runtime`) already in the script.
- [ ] Entitlements today: `network.client`, `files.user-selected.read-write`. Add `com.apple.security.personal-information.photos-library` only with the Photos feature.
- [ ] No GGUF weights in the pkg (~865 MB). First launch downloads them.

## 5. Build, sign, staple

```
IDENTITY_APP="Developer ID Application: Mayank Lavania (TEAMID)" \
IDENTITY_PKG="Developer ID Installer: Mayank Lavania (TEAMID)" \
APPLE_ID="you@domain" \
APPLE_TEAM_ID="TEAMID" \
APPLE_APP_PASSWORD="xxxx-xxxx-xxxx-xxxx" \
./scripts/package-macos.sh --sign --notarize
```

- [ ] `codesign --verify --deep --strict dist/Loupe.app`
- [ ] `spctl --assess --type install dist/Loupe-0.1.0.pkg`
- [ ] Staple the pkg (`xcrun stapler staple`).
- [ ] Fresh user account or a second Mac: double-click the pkg, no right-click Open, menu-bar icon appears, first-run download completes, a search returns photos.

## 6. Distribute

- [x] GitHub Release on [lavs9/loupe](https://github.com/lavs9/loupe) with the unsigned v0.1.0 `.pkg`.
- [ ] Later release with the stapled, notarized `.pkg`.
- [ ] Update feed JSON `{version,url}` pointed at that release page. The app opens the page; it does not replace the binary yet.
- [ ] Sparkle or a signed delta is a later version.

## 7. Not this version

- Mac App Store / sandbox. User-selected folders plus `.photo_index` on external disks fights the sandbox. Developer ID outside the store is the path.
- Intel Mac. llama.cpp is pinned to Apple Silicon.
- Photos.app library (see issue #2).

## 8. Checks before a human gets the pkg

- [ ] `scripts/e2e-gui.sh` PASS (isolated state, no download, no llama start)
- [ ] Idle 45 s: footprint and leak bytes stable
- [ ] First-run on a clean Application Support: banner reaches 100 %, then a search returns photos
- [ ] External disk: Index on the volume, unplug, plug into a second Mac, search hits without re-index

## Cost and time (order of magnitude)

| Item | Cost | Wait |
|---|---|---|
| Developer Program | $99 / year | 24–48 h after payment, sometimes longer for new accounts |
| Certificates | $0 | minutes once the account is active |
| First notarization | $0 | a few minutes per pkg |
| Icon + plist polish | engineering | half a day |
| Two-Mac disk check | hardware | whenever the drive arrives |
