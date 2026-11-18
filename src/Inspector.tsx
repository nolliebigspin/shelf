import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Album, Asset } from "./api";
import { formatBytes, formatDate, formatDuration } from "./format";

export interface InspectorActions {
  rename: (id: number, name: string) => void;
  rate: (rating: number) => void;
  addTags: (names: string[]) => void;
  removeTag: (name: string) => void;
  addToAlbum: (albumId: number) => void;
  reveal: (id: number) => void;
  openExternal: (id: number) => void;
  /** Only set while an album is open. */
  removeFromAlbum?: () => void;
  trash: () => void;
  restore: () => void;
  deleteFromDisk: () => void;
}

function Stars({ value, onChange }: { value: number; onChange: (n: number) => void }) {
  return (
    <div className="flex items-center text-lg leading-none">
      {[1, 2, 3, 4, 5].map((n) => (
        <button
          key={n}
          title={n === value ? "Clear rating" : `${n} star${n > 1 ? "s" : ""}`}
          onClick={() => onChange(n === value ? 0 : n)}
          className={`px-0.5 ${n <= value ? "text-amber-300" : "text-neutral-600 hover:text-neutral-400"}`}
        >
          ★
        </button>
      ))}
    </div>
  );
}

function NameField({ asset, onRename }: { asset: Asset; onRename: (name: string) => void }) {
  const [value, setValue] = useState(asset.display_name ?? "");
  useEffect(() => setValue(asset.display_name ?? ""), [asset.id, asset.display_name]);
  const commit = () => {
    if (value.trim() !== (asset.display_name ?? "")) onRename(value);
  };
  return (
    <input
      value={value}
      placeholder={asset.file_name}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          setValue(asset.display_name ?? "");
          e.currentTarget.blur();
        }
      }}
      className="w-full rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-neutral-100 outline-none select-text placeholder:text-neutral-600 focus:border-sky-500"
    />
  );
}

function TagInput({ onAdd }: { onAdd: (names: string[]) => void }) {
  const [value, setValue] = useState("");
  return (
    <input
      value={value}
      placeholder="Add tags, separated by commas"
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        if (e.key !== "Enter") return;
        const names = value.split(",").map((t) => t.trim()).filter(Boolean);
        if (names.length) onAdd(names);
        setValue("");
      }}
      className="w-full rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-neutral-100 outline-none select-text placeholder:text-neutral-600 focus:border-sky-500"
    />
  );
}

/** Containers the built-in player handles (H.264, HEVC, ProRes inside mp4/mov/m4v). */
export function isPlayable(asset: Asset): boolean {
  return (
    asset.kind === "video" &&
    asset.online &&
    !asset.missing &&
    asset.path !== null &&
    /\.(mp4|mov|m4v)$/i.test(asset.file_name)
  );
}

/**
 * Player for clips the webview can decode; otherwise a picture with a hint.
 * Always the full panel width, at the clip's own aspect ratio (the panel scrolls).
 */
function Preview({ asset }: { asset: Asset }) {
  const [failed, setFailed] = useState(false);
  const [fullFailed, setFullFailed] = useState(false);
  const poster = asset.thumb_path ? convertFileSrc(asset.thumb_path) : undefined;
  const aspectRatio = asset.width && asset.height ? `${asset.width} / ${asset.height}` : "16 / 9";
  if (isPlayable(asset) && !failed) {
    return (
      <video
        data-player
        src={convertFileSrc(asset.path!)}
        poster={poster}
        controls
        preload="metadata"
        onError={() => setFailed(true)}
        style={{ aspectRatio }}
        className="w-full shrink-0 rounded bg-black"
      />
    );
  }
  // Photos show the original file so a wide panel stays sharp; the thumbnail is the fallback.
  const original =
    asset.kind === "image" && asset.online && !asset.missing && asset.path && !fullFailed
      ? convertFileSrc(asset.path)
      : undefined;
  const src = original ?? poster;
  let hint: string | null = null;
  if (!asset.online) hint = "Drive offline";
  else if (asset.missing) hint = "File not found on last scan";
  else if (asset.kind === "video") hint = "Can’t play this format here. Use “Open externally”.";
  return (
    <div style={{ aspectRatio }} className="relative w-full shrink-0 overflow-hidden rounded bg-black">
      {src && (
        <img
          src={src}
          alt=""
          draggable={false}
          onError={() => original && setFullFailed(true)}
          className="h-full w-full object-contain"
        />
      )}
      {hint && (
        <div className="absolute inset-x-0 bottom-0 bg-black/70 px-2 py-1 text-center text-xs text-neutral-200">
          {hint}
        </div>
      )}
    </div>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-neutral-500">
      {children}
    </h3>
  );
}

export function Inspector({
  selection,
  albums,
  inTrash,
  actions,
}: {
  selection: Asset[];
  albums: Album[];
  inTrash: boolean;
  actions: InspectorActions;
}) {
  if (selection.length === 0) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-neutral-600">
        Select a clip to name, rate and tag it.
      </div>
    );
  }
  const single = selection.length === 1 ? selection[0] : null;
  const available = !!single && single.online && !single.missing;
  const rating = selection.every((a) => a.rating === selection[0].rating) ? selection[0].rating : 0;
  // Every tag used in the selection, with how many of the selected clips carry it.
  const tagCounts = new Map<string, number>();
  for (const a of selection) for (const t of a.tags) tagCounts.set(t, (tagCounts.get(t) ?? 0) + 1);
  const tags = [...tagCounts.entries()].sort((a, b) => a[0].localeCompare(b[0]));

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-3">
      {single ? (
        <>
          <Preview key={single.id} asset={single} />
          <div className="flex gap-2">
            <button
              disabled={!available}
              onClick={() => actions.reveal(single.id)}
              className="flex-1 rounded border border-neutral-700 px-2 py-1 text-neutral-200 hover:bg-neutral-800 disabled:text-neutral-600 disabled:hover:bg-transparent"
            >
              Reveal in Finder
            </button>
            <button
              disabled={!available}
              onClick={() => actions.openExternal(single.id)}
              className="flex-1 rounded border border-neutral-700 px-2 py-1 text-neutral-200 hover:bg-neutral-800 disabled:text-neutral-600 disabled:hover:bg-transparent"
            >
              Open externally
            </button>
          </div>
          {!inTrash && (
            <p className="-mt-2 text-[11px] leading-snug text-neutral-500">
              Drag clips onto an album or the Trash, between clips to rearrange (Sort: Custom), or out of
              the window into Final Cut, Premiere, Resolve or Finder (the original file, never a copy).
            </p>
          )}
          <section>
            <Label>Display name</Label>
            <NameField asset={single} onRename={(name) => actions.rename(single.id, name)} />
          </section>
        </>
      ) : (
        <div className="rounded bg-neutral-800 px-3 py-2 text-neutral-100">
          {selection.length.toLocaleString()} clips selected
        </div>
      )}

      <section>
        <Label>Rating</Label>
        <Stars value={rating} onChange={actions.rate} />
      </section>

      <section>
        <Label>Tags</Label>
        {tags.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-1">
            {tags.map(([name, count]) => (
              <span
                key={name}
                className="flex items-center gap-1 rounded-full bg-neutral-800 py-0.5 pl-2 pr-1 text-xs text-neutral-200"
              >
                {name}
                {!single && count < selection.length && (
                  <span className="text-neutral-500">{count}</span>
                )}
                <button
                  title={`Remove “${name}”`}
                  onClick={() => actions.removeTag(name)}
                  className="rounded-full px-1 text-neutral-500 hover:bg-neutral-700 hover:text-neutral-100"
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        )}
        <TagInput onAdd={actions.addTags} />
      </section>

      {inTrash ? (
        <section>
          <Label>In Trash</Label>
          <p className="mb-2 text-xs text-neutral-500">
            Hidden everywhere else. The file is still on its drive until you delete it from there.
          </p>
          <div className="flex gap-2">
            <button
              onClick={actions.restore}
              className="flex-1 rounded border border-neutral-700 px-2 py-1 text-neutral-200 hover:bg-neutral-800"
            >
              Put Back
            </button>
            <button
              onClick={actions.deleteFromDisk}
              className="flex-1 rounded border border-red-900 px-2 py-1 text-red-300 hover:bg-red-950"
            >
              Delete from Drive…
            </button>
          </div>
        </section>
      ) : (
      <section>
        <Label>Album</Label>
        <select
          value=""
          disabled={albums.length === 0}
          onChange={(e) => e.target.value && actions.addToAlbum(Number(e.target.value))}
          className="w-full rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-neutral-100 outline-none disabled:text-neutral-600"
        >
          <option value="">{albums.length === 0 ? "No albums yet" : "Add to album…"}</option>
          {albums.map((a) => (
            <option key={a.id} value={a.id}>
              {a.name}
            </option>
          ))}
        </select>
        {actions.removeFromAlbum && (
          <button
            onClick={actions.removeFromAlbum}
            className="mt-2 w-full rounded border border-neutral-700 px-2 py-1 text-neutral-300 hover:bg-neutral-800"
          >
            Remove from this album
          </button>
        )}
        <button
          onClick={actions.trash}
          title="Hide in Shelf’s Trash. Nothing changes on the drive. (⌘⌫)"
          className="mt-2 w-full rounded border border-neutral-700 px-2 py-1 text-neutral-300 hover:bg-neutral-800"
        >
          Move to Trash
        </button>
      </section>
      )}

      {single && (
        <section className="select-text">
          <Label>File</Label>
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            {(
              [
                ["Name", single.file_name],
                ["Drive", `${single.volume_label}${single.online ? "" : " (offline)"}`],
                ["Path", single.path ?? single.rel_path],
                ["Duration", single.duration_ms != null ? formatDuration(single.duration_ms) : null],
                ["Resolution", single.width && single.height ? `${single.width} × ${single.height}` : null],
                ["Codec", single.codec],
                ["Size", formatBytes(single.size_bytes)],
                ["Captured", single.created_at ? formatDate(single.created_at) : null],
                ["Status", single.missing ? "Not found on last scan" : null],
              ] as [string, string | null][]
            )
              .filter(([, v]) => v)
              .map(([k, v]) => (
                <div key={k} className="contents">
                  <dt className="text-neutral-500">{k}</dt>
                  <dd className="break-all text-neutral-300">{v}</dd>
                </div>
              ))}
          </dl>
        </section>
      )}
    </div>
  );
}
