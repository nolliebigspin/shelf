import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addRoot,
  addToAlbum,
  appStatus,
  createAlbum,
  listAlbums,
  listAssets,
  listTags,
  openExternal,
  refreshVolumes,
  removeFromAlbum,
  rescan,
  reveal,
  setTags,
  updateAsset,
  type Album,
  type AppStatus,
  type Asset,
  type AssetFilter,
  type ScanProgress,
  type Tag,
  type Volume,
} from "./api";
import { Grid } from "./Grid";
import { Inspector, type InspectorActions } from "./Inspector";
import { useSelection } from "./store";

type View =
  | { kind: "all" }
  | { kind: "volume"; id: number }
  | { kind: "album"; id: number }
  | { kind: "tag"; name: string };

function StatusLine({ status, error }: { status: AppStatus | null; error: string | null }) {
  if (error) return <span className="text-red-400">Backend error: {error}</span>;
  if (!status) return <span className="text-neutral-500">Starting…</span>;
  return (
    <>
      {status.ffmpeg_error ? (
        <span className="text-red-400">{status.ffmpeg_error}</span>
      ) : (
        <span>
          <span className="text-emerald-400">●</span> ffmpeg found: {status.ffmpeg} · ffprobe:{" "}
          {status.ffprobe}
        </span>
      )}
      <span className="ml-auto truncate text-neutral-500" title={status.db_path}>
        Catalog: {status.db_path}
      </span>
    </>
  );
}

function ScanStatus({ scans }: { scans: Record<number, ScanProgress> }) {
  const running = Object.values(scans).filter((s) => !s.done);
  if (running.length === 0) return null;
  const found = running.reduce((n, s) => n + s.found, 0);
  const total = running.reduce((n, s) => n + s.total, 0);
  const processed = running.reduce((n, s) => n + s.processed, 0);
  return (
    <div className="flex shrink-0 items-center gap-2 text-xs text-neutral-400">
      {total > 0 ? (
        <>
          <div className="h-1.5 w-40 overflow-hidden rounded-full bg-neutral-800">
            <div
              className="h-full bg-sky-400 transition-[width]"
              style={{ width: `${(processed / total) * 100}%` }}
            />
          </div>
          <span className="tabular-nums">
            Thumbnails {processed.toLocaleString()} / {total.toLocaleString()}
          </span>
        </>
      ) : (
        <>
          <span className="h-3 w-3 animate-spin rounded-full border-2 border-neutral-600 border-t-sky-400" />
          Scanning… {found.toLocaleString()} files found
        </>
      )}
    </div>
  );
}

function SidebarItem({
  active,
  onClick,
  title,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-neutral-800 ${
        active ? "bg-neutral-800 text-neutral-100" : ""
      }`}
    >
      {children}
    </button>
  );
}

function SidebarHeading({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="mb-1 px-2 text-[11px] font-semibold uppercase tracking-wide text-neutral-500">
      {children}
    </h2>
  );
}

const Count = ({ children }: { children: React.ReactNode }) => (
  <span className="ml-auto shrink-0 tabular-nums text-neutral-500">{children}</span>
);

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [volumes, setVolumes] = useState<Volume[]>([]);
  const [albums, setAlbums] = useState<Album[]>([]);
  const [tags, setTagList] = useState<Tag[]>([]);
  const [scans, setScans] = useState<Record<number, ScanProgress>>({});
  const [notice, setNotice] = useState<string | null>(null);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [view, setView] = useState<View>({ kind: "all" });
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [minRating, setMinRating] = useState(0);
  const [newAlbum, setNewAlbum] = useState<string | null>(null);
  const selected = useSelection((s) => s.selected);

  // Debounce typing so each keystroke doesn't hit the database.
  useEffect(() => {
    const t = setTimeout(() => setSearch(searchInput.trim()), 200);
    return () => clearTimeout(t);
  }, [searchInput]);

  const filter = useMemo<AssetFilter>(
    () => ({
      volume_id: view.kind === "volume" ? view.id : null,
      album_id: view.kind === "album" ? view.id : null,
      tag: view.kind === "tag" ? view.name : null,
      search: search || null,
      min_rating: minRating || null,
    }),
    [view, search, minRating],
  );
  const filterRef = useRef(filter);
  filterRef.current = filter;
  const lastReload = useRef(0);

  const loadAssets = useCallback(() => {
    lastReload.current = Date.now();
    const requested = filterRef.current;
    return listAssets(requested).then(
      (rows) => {
        // Ignore answers to a filter the user has already moved on from.
        if (filterRef.current !== requested) return;
        setAssets(rows);
        useSelection.getState().retain(rows.map((a) => a.id));
      },
      (e) => setNotice(String(e)),
    );
  }, []);

  const loadSidebar = useCallback(
    () =>
      Promise.all([listAlbums(), listTags()]).then(
        ([a, t]) => {
          setAlbums(a);
          setTagList(t);
        },
        (e) => setNotice(String(e)),
      ),
    [],
  );

  useEffect(() => {
    loadAssets();
  }, [filter, loadAssets]);

  // Which drives are mounted, as of the last poll. When it changes (eject / plug in),
  // the grid is reloaded so Offline badges and file paths follow.
  const mountState = useRef<string | null>(null);
  const loadVolumes = useCallback(
    () =>
      refreshVolumes().then(
        (vols) => {
          setVolumes(vols);
          const state = vols.map((v) => `${v.id}:${v.mount_path ?? ""}`).join("|");
          if (mountState.current !== null && mountState.current !== state) loadAssets();
          mountState.current = state;
        },
        (e) => setNotice(String(e)),
      ),
    [loadAssets],
  );

  useEffect(() => {
    const timer = setInterval(loadVolumes, 5000);
    return () => clearInterval(timer);
  }, [loadVolumes]);

  useEffect(() => {
    appStatus().then(setStatus, (e) => setError(String(e)));
    loadVolumes();
    loadSidebar();
    const unlisten = listen<ScanProgress>("scan_progress", ({ payload }) => {
      setScans((s) => ({ ...s, [payload.volume_id]: payload }));
      if (payload.error) setNotice(`Scan failed: ${payload.error}`);
      if (payload.done) {
        loadVolumes();
        loadAssets();
      } else if (Date.now() - lastReload.current > 2000) {
        // Let thumbnails trickle into the grid while the scan runs.
        loadAssets();
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [loadVolumes, loadAssets, loadSidebar]);

  const addFolder = async () => {
    const path = await open({ directory: true, title: "Add a folder or drive to the catalog" });
    if (!path) return;
    setNotice(null);
    try {
      await addRoot(path);
      await loadVolumes();
    } catch (e) {
      setNotice(String(e));
    }
  };

  /** Run a catalog change, then refresh everything it could have touched. */
  const mutate = useCallback(
    async (change: () => Promise<unknown>) => {
      setNotice(null);
      try {
        await change();
      } catch (e) {
        setNotice(String(e));
      }
      await Promise.all([loadAssets(), loadSidebar()]);
    },
    [loadAssets, loadSidebar],
  );

  const selectedIds = useCallback(() => [...useSelection.getState().selected], []);
  const rate = useCallback(
    (rating: number) => {
      const ids = selectedIds();
      if (ids.length === 0) return;
      // Show the stars at once; the reload confirms them.
      setAssets((rows) => rows.map((a) => (ids.includes(a.id) ? { ...a, rating } : a)));
      mutate(() => Promise.all(ids.map((id) => updateAsset(id, { rating }))));
    },
    [mutate, selectedIds],
  );

  const albumId = view.kind === "album" ? view.id : null;
  const actions: InspectorActions = {
    rename: (id, name) => mutate(() => updateAsset(id, { display_name: name })),
    rate,
    addTags: (names) => mutate(() => setTags(selectedIds(), names, "add")),
    removeTag: (name) => mutate(() => setTags(selectedIds(), [name], "remove")),
    addToAlbum: (id) => mutate(() => addToAlbum(id, selectedIds())),
    reveal: (id) => reveal(id).catch((e) => setNotice(String(e))),
    openExternal: (id) => openExternal(id).catch((e) => setNotice(String(e))),
    removeFromAlbum:
      albumId === null ? undefined : () => mutate(() => removeFromAlbum(albumId, selectedIds())),
  };

  const submitNewAlbum = () => {
    const name = (newAlbum ?? "").trim();
    setNewAlbum(null);
    if (!name) return;
    mutate(async () => {
      const id = await createAlbum(name);
      const ids = selectedIds();
      // "New album" with clips selected puts them straight in.
      if (ids.length > 0) await addToAlbum(id, ids);
    });
  };

  // Keyboard: 1–5 rate, 0 clear, Space play/pause, ⌘A select all, Esc deselect. Ignored while typing.
  const assetsRef = useRef(assets);
  assetsRef.current = assets;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT")) return;
      if (e.metaKey && e.key.toLowerCase() === "a") {
        e.preventDefault();
        useSelection.getState().selectAll(assetsRef.current.map((a) => a.id));
      } else if (e.key === " ") {
        // Space plays / pauses the clip in the right panel.
        const player = document.querySelector<HTMLVideoElement>("video[data-player]");
        if (!player) return;
        e.preventDefault();
        if (el?.tagName === "BUTTON") el.blur();
        if (player.paused) player.play().catch(() => {});
        else player.pause();
      } else if (e.key === "Escape") {
        useSelection.getState().clear();
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && /^[0-5]$/.test(e.key)) {
        e.preventDefault();
        rate(Number(e.key));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rate]);

  const total = volumes.reduce((n, v) => n + v.asset_count, 0);
  const selection = useMemo(() => assets.filter((a) => selected.has(a.id)), [assets, selected]);
  const filtered = view.kind !== "all" || search !== "" || minRating > 0;

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-56 shrink-0 flex-col gap-4 overflow-y-auto border-r border-neutral-800 bg-neutral-900 p-3">
          <SidebarItem active={view.kind === "all"} onClick={() => setView({ kind: "all" })}>
            All clips <Count>{total.toLocaleString()}</Count>
          </SidebarItem>

          <section>
            <SidebarHeading>Drives</SidebarHeading>
            {volumes.map((v) => {
              const scanning = !!scans[v.id] && !scans[v.id].done;
              return (
                <div key={v.id} className="flex items-center">
                  <div className="min-w-0 flex-1">
                    <SidebarItem
                      active={view.kind === "volume" && view.id === v.id}
                      onClick={() => setView({ kind: "volume", id: v.id })}
                      title={v.mount_path ?? "Offline"}
                    >
                      <span
                        className={`h-2 w-2 shrink-0 rounded-full ${v.online ? "bg-emerald-400" : "bg-neutral-600"}`}
                      />
                      <span className="truncate">{v.label}</span>
                      <Count>{v.asset_count.toLocaleString()}</Count>
                    </SidebarItem>
                  </div>
                  <button
                    disabled={!v.online || scanning}
                    onClick={() => {
                      setNotice(null);
                      rescan(v.id).catch((e) => setNotice(String(e)));
                    }}
                    title={
                      !v.online ? "Drive offline" : scanning ? "Scan running" : "Rescan the folders added on this drive"
                    }
                    className="ml-1 shrink-0 rounded px-1.5 py-1 text-neutral-500 hover:bg-neutral-800 hover:text-neutral-100 disabled:text-neutral-700 disabled:hover:bg-transparent"
                  >
                    ↻
                  </button>
                </div>
              );
            })}
            <button
              onClick={addFolder}
              className="mt-1 w-full rounded px-2 py-1 text-left text-sky-400 hover:bg-neutral-800"
            >
              + Add folder
            </button>
          </section>

          <section>
            <SidebarHeading>Albums</SidebarHeading>
            {albums.map((a) => (
              <SidebarItem
                key={a.id}
                active={view.kind === "album" && view.id === a.id}
                onClick={() => setView({ kind: "album", id: a.id })}
              >
                <span className="truncate">{a.name}</span>
                <Count>{a.asset_count.toLocaleString()}</Count>
              </SidebarItem>
            ))}
            {newAlbum === null ? (
              <button
                onClick={() => setNewAlbum("")}
                title={selected.size > 0 ? "Create an album with the selected clips" : "Create an empty album"}
                className="mt-1 w-full rounded px-2 py-1 text-left text-sky-400 hover:bg-neutral-800"
              >
                + New
              </button>
            ) : (
              <input
                autoFocus
                value={newAlbum}
                placeholder="Album name"
                onChange={(e) => setNewAlbum(e.target.value)}
                onBlur={submitNewAlbum}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.currentTarget.blur();
                  if (e.key === "Escape") setNewAlbum(null);
                }}
                className="mt-1 w-full rounded border border-sky-500 bg-neutral-950 px-2 py-1 text-neutral-100 outline-none select-text"
              />
            )}
          </section>

          <section>
            <SidebarHeading>Tags</SidebarHeading>
            {tags.length === 0 && <div className="px-2 text-neutral-600">No tags yet</div>}
            {tags.map((t) => (
              <SidebarItem
                key={t.id}
                active={view.kind === "tag" && view.name === t.name}
                onClick={() => setView({ kind: "tag", name: t.name })}
              >
                <span className="truncate">{t.name}</span>
                <Count>{t.asset_count.toLocaleString()}</Count>
              </SidebarItem>
            ))}
          </section>
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-10 shrink-0 items-center gap-3 border-b border-neutral-800 px-3">
            <input
              type="search"
              value={searchInput}
              placeholder="Search names and tags"
              onChange={(e) => setSearchInput(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && setSearchInput("")}
              className="w-64 rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-neutral-100 outline-none select-text placeholder:text-neutral-600 focus:border-sky-500"
            />
            <select
              value={minRating}
              onChange={(e) => setMinRating(Number(e.target.value))}
              title="Minimum rating"
              className="rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-neutral-100 outline-none"
            >
              <option value={0}>Any rating</option>
              {[1, 2, 3, 4, 5].map((n) => (
                <option key={n} value={n}>
                  {"★".repeat(n)}
                  {n < 5 ? " and up" : ""}
                </option>
              ))}
            </select>
            <ScanStatus scans={scans} />
            {notice && (
              <span className="min-w-0 truncate text-xs text-red-400" title={notice}>
                {notice}
              </span>
            )}
            <span className="ml-auto shrink-0 text-xs tabular-nums text-neutral-500">
              {selected.size > 0 && `${selected.size.toLocaleString()} selected · `}
              {assets.length.toLocaleString()} clips
            </span>
          </header>
          <main className="min-h-0 flex-1">
            {assets.length === 0 ? (
              <div className="flex h-full items-center justify-center text-neutral-500">
                {filtered
                  ? "Nothing here matches."
                  : "No clips yet. Use “+ Add folder” to scan a folder or drive."}
              </div>
            ) : (
              <Grid assets={assets} />
            )}
          </main>
        </div>

        <aside className="w-72 shrink-0 border-l border-neutral-800 bg-neutral-900">
          <Inspector selection={selection} albums={albums} actions={actions} />
        </aside>
      </div>
      <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-neutral-800 bg-neutral-900 px-3 text-xs text-neutral-400">
        <StatusLine status={status} error={error} />
      </footer>
    </div>
  );
}
