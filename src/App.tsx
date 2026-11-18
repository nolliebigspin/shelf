import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import {
  addRoot,
  addToAlbum,
  appStatus,
  createAlbum,
  deleteFromDisk,
  dragIcon,
  listAlbums,
  listAssets,
  listTags,
  moveAssets,
  openExternal,
  refreshVolumes,
  removeFromAlbum,
  rescan,
  restoreAssets,
  reveal,
  setTags,
  trashAssets,
  trashCount,
  updateAsset,
  type Album,
  type AppStatus,
  type Asset,
  type AssetFilter,
  type DeleteReport,
  type OrderScope,
  type ScanProgress,
  type SortKey,
  type Tag,
  type Volume,
} from "./api";
import { ClipList, Grid, gridMetrics } from "./Grid";
import { useDnd, type DragConfig, type DropTarget } from "./dnd";
import { formatBytes } from "./format";
import { Inspector, type InspectorActions } from "./Inspector";
import { clamp, ResizeHandle, usePanelWidth, useWindowWidth } from "./ResizeHandle";
import { showClipMenu } from "./contextMenu";
import { useSelection } from "./store";
import { DragGhost, ShortcutsHelp } from "./Overlays";
import {
  defaultSort,
  DESC_FIRST,
  readJson,
  sortKeysFor,
  SORT_LABELS,
  viewKey,
  writeJson,
  type Layout,
  type Sort,
  type View,
} from "./view";

const LEFT = { initial: 224, min: 180, max: 400 };
const RIGHT = { initial: 288, min: 260 };
/** The grid keeps at least this much room between the panels. */
const MIN_GRID = 320;

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
  drop,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title?: string;
  /** Makes the item a drop target for clips dragged from the grid. */
  drop?: { id: string; dragging: boolean; hovered: boolean };
  children: React.ReactNode;
}) {
  const dropStyle = !drop?.dragging
    ? ""
    : drop.hovered
      ? "bg-sky-500/25 text-neutral-100 ring-1 ring-sky-400"
      : "outline-1 outline-dashed -outline-offset-1 outline-neutral-600";
  return (
    <button
      onClick={onClick}
      title={title}
      data-drop={drop?.id}
      className={`flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-neutral-800 ${
        active ? "bg-neutral-800 text-neutral-100" : ""
      } ${dropStyle}`}
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
  /** Neutral feedback, e.g. after deleting from a drive. */
  const [info, setInfo] = useState<string | null>(null);
  const [trashTotal, setTrashTotal] = useState(0);
  const dropTarget = useDnd((s) => s.target);
  const draggingClips = useDnd((s) => s.drag !== null);
  const windowWidth = useWindowWidth();
  const [leftRaw, setLeft] = usePanelWidth("shelf.leftWidth", LEFT.initial);
  const [rightRaw, setRight] = usePanelWidth("shelf.rightWidth", RIGHT.initial);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [view, setView] = useState<View>({ kind: "all" });
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [minRating, setMinRating] = useState(0);
  const [newAlbum, setNewAlbum] = useState<string | null>(null);
  const selected = useSelection((s) => s.selected);
  const [layout, setLayoutState] = useState<Layout>(() =>
    readJson<Layout>("shelf.layout", "grid") === "list" ? "list" : "grid",
  );
  const setLayout = (l: Layout) => {
    setLayoutState(l);
    writeJson("shelf.layout", l);
  };
  // Sort is remembered per view (each album and drive has its own).
  const [sorts, setSorts] = useState<Record<string, Sort>>(() => readJson("shelf.sorts", {}));
  const stored = sorts[viewKey(view)];
  const sort: Sort = stored && sortKeysFor(view).includes(stored.key) ? stored : defaultSort(view);
  const setSort = (next: Sort) => {
    const all = { ...sorts, [viewKey(view)]: next };
    setSorts(all);
    writeJson("shelf.sorts", all);
  };
  const sortBy = (key: SortKey) =>
    setSort(key === sort.key ? { key, desc: !sort.desc } : { key, desc: DESC_FIRST.includes(key) });

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
      trashed: view.kind === "trash" ? true : null,
      sort: sort.key,
      descending: sort.desc,
    }),
    [view, search, minRating, sort.key, sort.desc],
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
      Promise.all([listAlbums(), listTags(), trashCount()]).then(
        ([a, t, n]) => {
          setAlbums(a);
          setTagList(t);
          setTrashTotal(n);
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
      setInfo(null);
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

  /** Trash membership changes drive counts too, which come from the volume list. */
  const mutateTrash = useCallback(
    (change: () => Promise<unknown>) => mutate(change).then(() => loadVolumes()),
    [mutate, loadVolumes],
  );

  const assetsRef = useRef(assets);
  assetsRef.current = assets;

  /** Ask, then permanently delete these trashed clips' files from their drives. */
  const confirmDelete = useCallback(
    async (rows: Asset[]) => {
      if (rows.length === 0) return;
      setNotice(null);
      setInfo(null);
      const reachable = rows.filter((a) => a.online);
      const offline = rows.length - reachable.length;
      if (reachable.length === 0) {
        setNotice("Their drive is offline. Plug it in to delete these files.");
        return;
      }
      const bytes = reachable.reduce((n, a) => n + a.size_bytes, 0);
      const files = reachable.length === 1 ? "1 file" : `${reachable.length.toLocaleString()} files`;
      const ok = await ask(
        `Permanently delete ${files} (${formatBytes(bytes)}) from your drives?\n\n` +
          "They will not go to the Finder Trash. This cannot be undone." +
          (offline > 0 ? `\n\n${offline.toLocaleString()} on offline drives will be skipped.` : ""),
        { title: "Delete from Drive", kind: "warning", okLabel: "Delete Permanently", cancelLabel: "Cancel" },
      );
      if (!ok) return;
      let report: DeleteReport | null = null;
      await mutateTrash(async () => {
        report = await deleteFromDisk(reachable.map((a) => a.id));
      });
      const r = report as DeleteReport | null;
      if (!r) return;
      const parts = [`Deleted ${r.deleted.toLocaleString()} from drive`];
      if (offline + r.skipped_offline > 0) parts.push(`${offline + r.skipped_offline} skipped (offline)`);
      setInfo(parts.join(" · "));
      if (r.failed.length > 0) setNotice(`${r.failed.length} could not be deleted: ${r.failed.join("; ")}`);
    },
    [mutateTrash],
  );

  const emptyTrash = useCallback(
    () => listAssets({ trashed: true }).then(confirmDelete, (e) => setNotice(String(e))),
    [confirmDelete],
  );

  const trashSelected = useCallback(() => {
    const ids = selectedIds();
    if (ids.length > 0) mutateTrash(() => trashAssets(ids));
  }, [mutateTrash, selectedIds]);

  const albumId = view.kind === "album" ? view.id : null;
  const inTrash = view.kind === "trash";
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
    trash: trashSelected,
    restore: () => mutateTrash(() => restoreAssets(selectedIds())),
    deleteFromDisk: () => {
      const ids = new Set(selectedIds());
      confirmDelete(assetsRef.current.filter((a) => ids.has(a.id)));
    },
  };
  const actionsRef = useRef(actions);
  actionsRef.current = actions;
  const albumsRef = useRef(albums);
  albumsRef.current = albums;
  const albumName = albumId === null ? null : (albums.find((a) => a.id === albumId)?.name ?? null);
  const menuContext = useRef({ inTrash, albumName });
  menuContext.current = { inTrash, albumName };

  const openClipMenu = useCallback(() => {
    const { selected } = useSelection.getState();
    showClipMenu({
      selection: assetsRef.current.filter((a) => selected.has(a.id)),
      albums: albumsRef.current,
      ...menuContext.current,
      actions: actionsRef.current,
    }).catch((e) => setNotice(String(e)));
  }, []);

  // Drag and drop inside the window (see dnd.ts): onto an album, the Trash, or
  // between clips to rearrange. Leaving the window hands over to a native file drag.
  const orderScope: OrderScope | null =
    view.kind === "album"
      ? { album_id: view.id, volume_id: null }
      : view.kind === "volume"
        ? { album_id: null, volume_id: view.id }
        : view.kind === "all"
          ? { album_id: null, volume_id: null }
          : null;
  const reorderBlocker =
    orderScope === null
      ? "This view can’t be arranged by hand"
      : sort.key !== "custom" || sort.desc
        ? "Choose Sort: Custom ↑ to rearrange"
        : search || minRating
          ? "Clear the search and rating filter to rearrange"
          : null;

  const dropOn = (ids: number[], target: DropTarget) => {
    if (target.kind === "album") mutate(() => addToAlbum(target.id, ids));
    else if (target.kind === "trash") mutateTrash(() => trashAssets(ids));
    else if (target.kind === "reorder" && orderScope) {
      // Show the new order at once; the reload confirms it.
      const moving = new Set(ids);
      setAssets((rows) => {
        const moved = rows.filter((a) => moving.has(a.id));
        const rest = rows.filter((a) => !moving.has(a.id));
        const at = target.beforeId === null ? -1 : rest.findIndex((a) => a.id === target.beforeId);
        rest.splice(at < 0 ? rest.length : at, 0, ...moved);
        return rest;
      });
      mutate(() => moveAssets(orderScope, ids, target.beforeId));
    }
  };

  /** Hand the drag to macOS once the pointer leaves the window, for editors and Finder. */
  const dragOut = async (ids: number[]) => {
    const wanted = new Set(ids);
    const files = assetsRef.current.filter((a) => wanted.has(a.id) && a.online && !a.missing && a.path);
    if (files.length === 0) {
      setNotice("Only clips on a connected drive can be dragged out of Shelf.");
      return;
    }
    const icon = await dragIcon(files.map((a) => a.id)).catch(() => files.find((a) => a.thumb_path)?.thumb_path);
    if (!icon) return;
    // "copy" so a drop onto Finder can never move the original off the drive.
    startDrag({ item: files.map((a) => a.path!), icon, mode: "copy" }).catch((e) => setNotice(String(e)));
  };

  const dragConfig: DragConfig = {
    order: () => assetsRef.current.map((a) => a.id),
    layout,
    canReorder: () => reorderBlocker === null,
    canDropOnSidebar: () => !inTrash,
    describe: (target, ids) => {
      if (!target) return null;
      if (target.kind === "album") {
        const name = albumsRef.current.find((a) => a.id === target.id)?.name ?? "album";
        return `Add to “${name}”`;
      }
      if (target.kind === "trash") return ids.length > 1 ? `Move ${ids.length} to Trash` : "Move to Trash";
      if (target.kind === "reorder") return "Move here";
      return reorderBlocker;
    },
    onDrop: dropOn,
    onLeaveWindow: (ids) => void dragOut(ids),
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

  // Keyboard shortcuts (listed in SHORTCUTS / the "?" overlay). Ignored while typing.
  // ⌘⌫ never deletes from a drive: it trashes, or puts back inside the Trash.
  const [showKeys, setShowKeys] = useState(false);
  const keyState = useRef({ inTrash, layout, showKeys, actions });
  keyState.current = { inTrash, layout, showKeys, actions };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT")) return;
      const { inTrash, layout, showKeys, actions } = keyState.current;
      const key = e.key.toLowerCase();
      const order = () => assetsRef.current.map((a) => a.id);
      const sel = useSelection.getState();
      const single = sel.selected.size === 1 ? assetsRef.current.find((a) => sel.selected.has(a.id)) : undefined;
      const arrows: Record<string, number> = {
        ArrowLeft: layout === "grid" ? -1 : 0,
        ArrowRight: layout === "grid" ? 1 : 0,
        ArrowUp: layout === "grid" ? -gridMetrics.columns : -1,
        ArrowDown: layout === "grid" ? gridMetrics.columns : 1,
        Home: -Infinity,
        End: Infinity,
      };
      if (showKeys) {
        if (e.key === "Escape" || e.key === "?") {
          e.preventDefault();
          setShowKeys(false);
        }
        return;
      }
      if (e.key in arrows && !e.altKey && !e.ctrlKey) {
        let delta = arrows[e.key];
        if (e.metaKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) delta = e.key === "ArrowUp" ? -Infinity : Infinity;
        if (delta === 0) return;
        e.preventDefault();
        if (el?.tagName === "BUTTON") el.blur();
        sel.step(order(), delta, e.shiftKey);
      } else if (e.key === "?") {
        e.preventDefault();
        setShowKeys(true);
      } else if (e.key === "Enter" && !e.metaKey && single) {
        if (el?.tagName === "BUTTON") return; // Return activates a focused button as usual
        e.preventDefault();
        const field = document.getElementById("display-name") as HTMLInputElement | null;
        field?.focus();
        field?.select();
      } else if (e.metaKey && key === "o") {
        e.preventDefault();
        if (single?.online && !single.missing) actions.openExternal(single.id);
      } else if (e.metaKey && key === "r") {
        e.preventDefault();
        if (single?.online && !single.missing) actions.reveal(single.id);
      } else if (e.metaKey && key === "f") {
        e.preventDefault();
        const search = document.getElementById("search") as HTMLInputElement | null;
        search?.focus();
        search?.select();
      } else if (e.metaKey && (e.key === "1" || e.key === "2")) {
        e.preventDefault();
        setLayout(e.key === "1" ? "grid" : "list");
      } else if (e.metaKey && key === "a") {
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
      } else if ((e.key === "Backspace" && e.metaKey) || e.key === "Delete") {
        e.preventDefault();
        if (sel.selected.size === 0) return;
        if (inTrash) actions.restore();
        else trashSelected();
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && /^[0-5]$/.test(e.key)) {
        e.preventDefault();
        rate(Number(e.key));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rate, trashSelected]);

  const total = volumes.reduce((n, v) => n + v.asset_count, 0);
  const selection = useMemo(() => assets.filter((a) => selected.has(a.id)), [assets, selected]);
  const filtered = view.kind !== "all" || search !== "" || minRating > 0;
  const left = clamp(leftRaw, LEFT.min, LEFT.max);
  const right = clamp(rightRaw, RIGHT.min, windowWidth - left - MIN_GRID);
  const dropProps = (id: string) => ({
    id,
    dragging: draggingClips && !inTrash,
    hovered:
      (dropTarget?.kind === "trash" && id === "trash") ||
      (dropTarget?.kind === "album" && id === `album:${dropTarget.id}`),
  });

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <aside
          style={{ width: left }}
          className="flex shrink-0 flex-col gap-4 overflow-y-auto border-r border-neutral-800 bg-neutral-900 p-3"
        >
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
                drop={dropProps(`album:${a.id}`)}
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

          <section className="mt-auto">
            <SidebarItem
              active={view.kind === "trash"}
              onClick={() => setView({ kind: "trash" })}
              title="Clips hidden from the catalog. Their files stay on the drive until you delete them here."
              drop={dropProps("trash")}
            >
              <span aria-hidden className="shrink-0 text-neutral-500">
                🗑
              </span>
              Trash <Count>{trashTotal > 0 ? trashTotal.toLocaleString() : ""}</Count>
            </SidebarItem>
          </section>
        </aside>
        <ResizeHandle
          side="left"
          width={left}
          min={LEFT.min}
          max={LEFT.max}
          onChange={setLeft}
          onReset={() => setLeft(LEFT.initial)}
        />

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-10 shrink-0 items-center gap-3 border-b border-neutral-800 px-3">
            <input
              type="search"
              value={searchInput}
              placeholder="Search names and tags"
              onChange={(e) => setSearchInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  if (searchInput) setSearchInput("");
                  else e.currentTarget.blur();
                } else if (e.key === "ArrowDown" || e.key === "Enter") {
                  // Leave the field and continue with the arrow keys in the clips.
                  e.preventDefault();
                  e.currentTarget.blur();
                  useSelection.getState().step(assetsRef.current.map((a) => a.id), 1, false);
                }
              }}
              id="search"
              className="control w-64 min-w-32 shrink select-text placeholder:text-neutral-600"
            />
            <select
              value={minRating}
              onChange={(e) => setMinRating(Number(e.target.value))}
              title="Minimum rating"
              aria-label="Minimum rating"
              className="control control-select shrink-0"
            >
              <option value={0}>Any rating</option>
              {[1, 2, 3, 4, 5].map((n) => (
                <option key={n} value={n}>
                  {"★".repeat(n)}
                  {n < 5 ? " and up" : ""}
                </option>
              ))}
            </select>
            <div className="flex shrink-0 items-center">
              <select
                value={sort.key}
                onChange={(e) => setSort({ key: e.target.value as SortKey, desc: DESC_FIRST.includes(e.target.value as SortKey) })}
                title="Sort by"
                aria-label="Sort by"
                className="control control-select relative rounded-r-none focus:z-10"
              >
                {sortKeysFor(view).map((k) => (
                  <option key={k} value={k}>
                    {k === "custom" ? "Custom (drag to arrange)" : SORT_LABELS[k]}
                  </option>
                ))}
              </select>
              <button
                onClick={() => setSort({ ...sort, desc: !sort.desc })}
                title={sort.desc ? "Descending: click for ascending" : "Ascending: click for descending"}
                aria-label={sort.desc ? "Descending" : "Ascending"}
                className="control -ml-px w-7 rounded-l-none px-0 text-neutral-300 hover:text-neutral-100"
              >
                {sort.desc ? "↓" : "↑"}
              </button>
            </div>
            <div className="flex h-7 shrink-0 overflow-hidden rounded border border-neutral-700" role="group" aria-label="Layout">
              {(["grid", "list"] as const).map((l) => (
                <button
                  key={l}
                  onClick={() => setLayout(l)}
                  title={l === "grid" ? "Grid" : "List"}
                  aria-pressed={layout === l}
                  className={`flex h-full w-8 items-center justify-center ${layout === l ? "bg-neutral-700 text-neutral-100" : "bg-neutral-950 text-neutral-500 hover:text-neutral-200"}`}
                >
                  {l === "grid" ? (
                    <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor" aria-hidden>
                      <rect x="1" y="1" width="5" height="5" rx="1" />
                      <rect x="8" y="1" width="5" height="5" rx="1" />
                      <rect x="1" y="8" width="5" height="5" rx="1" />
                      <rect x="8" y="8" width="5" height="5" rx="1" />
                    </svg>
                  ) : (
                    <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor" aria-hidden>
                      <rect x="1" y="2" width="12" height="2" rx="1" />
                      <rect x="1" y="6" width="12" height="2" rx="1" />
                      <rect x="1" y="10" width="12" height="2" rx="1" />
                    </svg>
                  )}
                </button>
              ))}
            </div>
            <ScanStatus scans={scans} />
            {inTrash && (
              <>
                {selected.size > 0 && (
                  <button
                    onClick={actions.restore}
                    className="h-7 shrink-0 rounded border border-neutral-700 px-2 text-xs text-neutral-200 hover:bg-neutral-800"
                  >
                    Put Back
                  </button>
                )}
                {selected.size > 0 && (
                  <button
                    onClick={actions.deleteFromDisk}
                    className="h-7 shrink-0 rounded border border-red-900 px-2 text-xs text-red-300 hover:bg-red-950"
                  >
                    Delete Selected from Drive…
                  </button>
                )}
                <button
                  disabled={trashTotal === 0}
                  onClick={emptyTrash}
                  title="Permanently delete every clip in the Trash from its drive"
                  className="h-7 shrink-0 rounded border border-red-900 px-2 text-xs text-red-300 hover:bg-red-950 disabled:border-neutral-800 disabled:text-neutral-600 disabled:hover:bg-transparent"
                >
                  Empty Trash…
                </button>
              </>
            )}
            {notice && (
              <span className="min-w-0 truncate text-xs text-red-400" title={notice}>
                {notice}
              </span>
            )}
            {info && !notice && (
              <span className="min-w-0 truncate text-xs text-neutral-400" title={info}>
                {info}
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
                {inTrash
                  ? search || minRating
                    ? "Nothing in the Trash matches."
                    : "The Trash is empty. Right-click a clip or press ⌘⌫ to move it here."
                  : filtered
                    ? "Nothing here matches."
                    : "No clips yet. Use “+ Add folder” to scan a folder or drive."}
              </div>
            ) : (
              layout === "grid" ? (
                <Grid assets={assets} onContextMenu={openClipMenu} dragConfig={dragConfig} />
              ) : (
                <ClipList
                  assets={assets}
                  onContextMenu={openClipMenu}
                  dragConfig={dragConfig}
                  sort={sort}
                  onSort={sortBy}
                />
              )
            )}
          </main>
        </div>

        <ResizeHandle
          side="right"
          width={right}
          min={RIGHT.min}
          max={windowWidth - left - MIN_GRID}
          onChange={setRight}
          onReset={() => setRight(RIGHT.initial)}
        />
        <aside style={{ width: right }} className="shrink-0 border-l border-neutral-800 bg-neutral-900">
          <Inspector selection={selection} albums={albums} inTrash={inTrash} actions={actions} />
        </aside>
      </div>
      <DragGhost />
      {showKeys && <ShortcutsHelp onClose={() => setShowKeys(false)} />}
      <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-neutral-800 bg-neutral-900 px-3 text-xs text-neutral-400">
        <StatusLine status={status} error={error} />
        <button
          onClick={() => setShowKeys(true)}
          title="Keyboard shortcuts (?)"
          className="shrink-0 rounded border border-neutral-700 px-1.5 leading-4 text-neutral-400 hover:text-neutral-100"
        >
          ?
        </button>
      </footer>
    </div>
  );
}
