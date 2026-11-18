# Media Catalog App — Build Plan

Nov 17, 2026 · Alec Winter

## 1. Scope for today

A Mac-only app that scans a folder on your SSD, shows every clip as a thumbnail, and lets you name, tag, rate and group clips into albums. It never moves, renames or modifies a media file. Everything else waits.

**In (must work tonight)**

- Add a folder or a whole drive, then scan videos and photos recursively
- One thumbnail plus duration, resolution and codec per file (ffmpeg / ffprobe)
- Grid view; with the drive unplugged, thumbnails stay and clips show "Offline"
- Albums: create, add the selected clips, open an album
- Per clip: display name, tags, rating 1–5
- Search by file name, display name and tag
- Play a clip in the app (H.264, HEVC, ProRes); anything else gets "Open externally"
- "Reveal in Finder", and drag clips from the grid into Final Cut, Premiere or Resolve (the original file, nothing copied)

**Later:** subclips and markers, hover-scrub, proxies, NLE XML export, relinking after renames, smart albums, Windows, signed installers.

**Assumptions:** personal use on your Mac; ffmpeg installed with Homebrew rather than bundled.

## 2. Rules the agent must never break

1. **Read-only on media.** Never write, rename, move or delete anything on a scanned drive. Open media files read-only. "Remove" and "Move to Trash" in the UI change the catalog only. The single exception (added Nov 18, 2026, on request): **Trash → Delete from Drive / Empty Trash** permanently deletes the files of clips that are already in Shelf's Trash, after a warning dialog. Only `trash::delete_from_disk` may delete files, and it ignores any clip that isn't in the Trash.
2. **Locate files by drive UUID + relative path, never by absolute path.** `/Volumes/T7` can change; the UUID doesn't.
3. **Derived files live in the app data folder** (thumbnails, database), never next to the media.
4. **Offline is normal, not an error.** Unmounted drive = clips still listed, metadata still editable, playback disabled.
5. **Rust owns the filesystem and the database.** The frontend only calls commands and gets asset URLs back.
6. **Scanning runs in the background and reports progress;** the UI never freezes.

## 3. Stack

| Part | Choice |
| --- | --- |
| App shell | Tauri 2, `create-tauri-app` with the React + TypeScript template |
| UI | React, Tailwind, TanStack Virtual (grid), Zustand (selection state) |
| Database | SQLite via `rusqlite` (feature `bundled`), schema created on startup |
| Scanning | `walkdir`, run on a background thread |
| Drive UUID | `diskutil info -plist <mount>` parsed with the `plist` crate → `VolumeUUID` |
| Metadata + thumbnails | Homebrew `ffprobe` / `ffmpeg` via `std::process::Command` |
| Playback + thumbnails in UI | `<video>` / `<img>` with `convertFileSrc` (asset protocol) |
| Drag into editors | `tauri-plugin-drag` (drags the original file path out of the app) |
| Reveal in Finder / open externally | `tauri-plugin-opener` |

**Two gotchas to hand the agent up front**

- A Mac app started from Finder doesn't get your shell's PATH. Look for ffmpeg at `/opt/homebrew/bin` first, then `/usr/local/bin`, and show a clear error if it's missing.
- Enable the asset protocol (Cargo feature `protocol-asset`) with scope `/Volumes/**` plus the app data folder, or videos and thumbnails won't load.

## 4. Data model

Six tables in one file, `catalog.db`, in the app data folder. A clip is online when its drive's UUID is currently mounted; its full path is `mount_path + "/" + rel_path`.

```sql
CREATE TABLE volumes (
  id          INTEGER PRIMARY KEY,
  uuid        TEXT NOT NULL UNIQUE,   -- VolumeUUID from diskutil
  label       TEXT NOT NULL,          -- last seen drive name
  mount_path  TEXT,                   -- e.g. /Volumes/T7, NULL when unplugged
  last_seen_at INTEGER
);

CREATE TABLE assets (
  id           INTEGER PRIMARY KEY,
  volume_id    INTEGER NOT NULL REFERENCES volumes(id),
  rel_path     TEXT NOT NULL,         -- path inside the drive, '/' separators
  file_name    TEXT NOT NULL,
  kind         TEXT NOT NULL,         -- 'video' | 'image'
  size_bytes   INTEGER NOT NULL,
  mtime        INTEGER NOT NULL,
  duration_ms  INTEGER,
  width        INTEGER,
  height       INTEGER,
  codec        TEXT,
  created_at   INTEGER,               -- capture date from metadata, else mtime
  thumb_path   TEXT,                  -- in app data, NULL until generated
  display_name TEXT,                  -- your virtual name
  rating       INTEGER DEFAULT 0,     -- 0-5
  missing      INTEGER DEFAULT 0,     -- 1 = not found on last scan
  UNIQUE(volume_id, rel_path)
);

CREATE TABLE tags        (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE asset_tags  (asset_id INTEGER, tag_id INTEGER, PRIMARY KEY (asset_id, tag_id));
CREATE TABLE albums      (id INTEGER PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER);
CREATE TABLE album_items (album_id INTEGER, asset_id INTEGER, position INTEGER,
                          PRIMARY KEY (album_id, asset_id));
```

> **Added after step 7:** a seventh table, `roots(id, volume_id, rel_path)`, records the folders you added so `rescan(volume_id)` re-walks only those instead of the whole drive.

File types to scan: `mp4 mov m4v mxf mkv mts m2ts avi` and `jpg jpeg png heic`. Skip hidden files, `._*`, `.Spotlight-V100`, `.Trashes` and `.fseventsd`.

## 5. Backend commands and UI

### Tauri commands

| Command | What it does |
| --- | --- |
| `add_root(path)` | Resolve the drive UUID, upsert the volume, start a background scan, return at once |
| `rescan(volume_id)` | Scan that drive again |
| `refresh_volumes()` | Match mounted drives in `/Volumes` to known UUIDs, update `mount_path`; the UI polls it every 5 s |
| `list_assets(filter)` | Filter by album, search text, tag, minimum rating; returns rows with online, full path and thumbnail path |
| `update_asset(id, patch)` | Set display name and/or rating |
| `set_tags(asset_ids, names, add\|remove)` | Tag or untag a selection; creates missing tags |
| `list_tags()` / `list_albums()` | Sidebar data |
| `create_album(name)` | New empty album |
| `add_to_album(album_id, asset_ids)` / `remove_from_album(...)` | Album membership |
| `reveal(asset_id)` / `open_external(asset_id)` | Finder / default player |

Event `scan_progress { volume_id, found, processed, done }` drives the progress bar.

### Scan pipeline (per drive)

1. Walk the folder and upsert a row per media file on `(volume_id, rel_path)`, keeping your names, ratings and tags.
2. Mark rows that weren't seen as `missing = 1`.
3. For new or changed files (size or mtime differ), two at a time:
   - `ffprobe -v error -print_format json -show_format -show_streams <file>` → duration, size, codec, capture date
   - `ffmpeg -ss 1 -i <file> -frames:v 1 -vf scale=400:-2 -q:v 5 <appdata>/thumbs/<id>.jpg` (use `-ss 0` for clips under 1 s)

### One screen

- **Left sidebar:** All clips · Drives (green dot online, grey offline, "+ Add folder") · Albums ("+ New") · Tags
- **Top bar:** search field, minimum-rating filter, scan progress
- **Center:** virtualized thumbnail grid showing display name (or file name), duration and an "Offline" badge
- **Right panel:** player (if online and mp4/mov/m4v), display name field, stars, tag input, "Add to album", path and metadata, Reveal in Finder / Open externally
- **Keys and mouse:** click, ⌘-click and Shift-click to select · 1–5 rate, 0 clear · Space play · drag clips from the grid into your editor

## 6. Build order

Seven steps, roughly 6 hours with an agent doing the typing. Don't start a step until the previous checkpoint passes on your real SSD.

1. **Skeleton (~30 min)** — create-tauri-app, Tailwind, rusqlite with the schema, ffmpeg path detection.
   - Checkpoint: app opens, `catalog.db` exists, a status line says ffmpeg was found.
2. **Drive + scan (~1.5 h)** — folder picker (`tauri-plugin-dialog`), diskutil UUID, walk + upsert, progress event.
   - Checkpoint: adding your SSD gives the same file count as Finder; adding it again creates no duplicates.
3. **Metadata + thumbnails (~1 h)** — ffprobe and ffmpeg jobs, virtualized grid.
   - Checkpoint: 1,000+ clips scroll smoothly with thumbnails and durations.
4. **Offline (~30 min)** — `refresh_volumes` polling and badges.
   - Checkpoint: eject the SSD and "Offline" appears within 5 s while thumbnails stay; plug it back in and it goes online.
5. **Organize (~1.5 h)** — display names, ratings, tags, albums, search.
   - Checkpoint: quit and reopen the app; everything is still there.
6. **Play and use (~1 h)** — player, Reveal in Finder, Open externally, drag into editors.
   - Checkpoint: drag three clips from an album into a Resolve or Final Cut timeline; they link to the files on the SSD.
7. **Safety check (~15 min)** — list a test folder's names, sizes and modified dates before and after a full scan.
   - Checkpoint: both lists are identical.

If time runs short, drop drag-and-drop in step 6. Reveal in Finder gets the file into your editor too.

## 7. Kickoff prompt

Export this doc as Markdown, save it as `PLAN.md` in an empty folder, and start your agent there with:

```text
Read PLAN.md. Build the app it describes, one step from section 6 at a time.

Rules:
- Follow section 2 strictly. Never write to, rename, move or delete files on a scanned drive, including in tests (the Trash delete test works on a temp folder it creates).
- Use exactly the stack in section 3 and the schema in section 4. Don't add features from "Later".
- After each step, run the app, tell me how to verify its checkpoint, and stop until I confirm.
- Keep Rust in modules: db.rs, volumes.rs, scan.rs, media.rs (ffmpeg/ffprobe), commands.rs.
- If something in the plan doesn't work on this Mac, say what and propose the smallest fix before changing course.

Start with step 1.
```

Prompt for each following step: `Checkpoint N passed. Do step N+1.` If a checkpoint fails, paste the error or describe what you saw, and don't move on.
