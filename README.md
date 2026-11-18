# Shelf

A read-only media catalog for macOS: scan a folder or drive, browse clips as thumbnails, then name, tag, rate and group them into albums. Media files are never modified; the only exception is **Trash → Delete from Drive**, which permanently deletes trashed clips' files after a confirmation. See [PLAN.md](PLAN.md).

```bash
npm install
npm run tauri dev
```

Needs ffmpeg from Homebrew (`brew install ffmpeg`).

## Landing page

The download page is an Astro site in [site/](site). To deploy it on Vercel, import this repository and set the Root Directory to `site`; the defaults work. The download button always points at the newest GitHub release (`releases/latest/download/Shelf_universal.dmg`, a fixed-name copy the release workflow uploads), and the version label is read from the latest release, so a new release needs no site change.

```bash
npm --prefix site install
npm --prefix site run dev
```
