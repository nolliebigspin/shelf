# Shelf

A read-only media catalog for macOS, Windows and Linux: scan a folder or drive, browse clips as thumbnails, then name, tag, rate and group them into albums. Media files are never modified; the only exception is **Trash → Delete from Drive**, which permanently deletes trashed clips' files after a confirmation. See [PLAN.md](PLAN.md).

```bash
npm install
npm run tauri dev
```

Needs ffmpeg: `brew install ffmpeg` (macOS), `winget install Gyan.FFmpeg` (Windows) or `sudo apt install ffmpeg` (Linux).

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed: a signed and notarized universal macOS app, a Windows installer (NSIS and MSI), and Linux AppImage, .deb and .rpm packages. `.github/workflows/ci.yml` runs the tests on all three systems for every push to main and every pull request.

## Landing page

The download page is an Astro site in [site/](site). To deploy it on Vercel, import this repository and set the Root Directory to `site`; the defaults work. The download button always points at the newest GitHub release (fixed-name copies the release workflow uploads, e.g. `releases/latest/download/Shelf_universal.dmg`, `Shelf_windows_x64-setup.exe`, `Shelf_linux_amd64.AppImage`), and the version label is read from the latest release, so a new release needs no site change.

```bash
npm --prefix site install
npm --prefix site run dev
```
