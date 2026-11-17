# Shelf

A read-only media catalog for macOS: scan a folder or drive, browse clips as thumbnails, then name, tag, rate and group them into albums. Media files are never modified. See [PLAN.md](PLAN.md).

```bash
npm install
npm run tauri dev
```

Needs ffmpeg from Homebrew (`brew install ffmpeg`).

## Landing page

The download page is an Astro site in [site/](site). To deploy it on Vercel, import this repository and set the Root Directory to `site`; the defaults work. The version and download link live in `site/src/release.ts`.

```bash
npm --prefix site install
npm --prefix site run dev
```
