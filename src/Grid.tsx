import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Asset, SortKey } from "./api";
import { trackPointer, useDnd, type DragConfig } from "./dnd";
import { formatBytes, formatDate, formatDuration } from "./format";
import { primary } from "./platform";
import { useSelection } from "./store";

const MIN_CARD = 190; // px, cards stretch to fill the row
const GAP = 10;
const PAD = 12;
const LABEL_H = 26;
const ROW_H = 44; // list layout

/** Cards per row in the grid right now, so ↑ / ↓ can move a whole row. */
export const gridMetrics = { columns: 1 };

/** Scroll the clip the keyboard just moved to into view. */
function useScrollToFocus(assets: Asset[], scrollTo: (index: number) => void) {
  const focus = useSelection((s) => s.focus);
  const assetsRef = useRef(assets);
  assetsRef.current = assets;
  const scrollRef = useRef(scrollTo);
  scrollRef.current = scrollTo;
  useEffect(() => {
    if (!focus) return;
    const i = assetsRef.current.findIndex((a) => a.id === focus.id);
    if (i >= 0) scrollRef.current(i);
  }, [focus]);
}

interface ClipViewProps {
  assets: Asset[];
  /** Right-click on a clip; the clip is selected first unless it already is. */
  onContextMenu: () => void;
  dragConfig: DragConfig;
}

type ItemHandlers = {
  onSelect: (id: number, e: React.MouseEvent) => void;
  onPointerDown: (id: number, e: React.PointerEvent) => void;
  onMenu: (id: number) => void;
};

/** Selection, right-click and drag handling shared by the grid and the list. */
function useItemHandlers(
  assets: Asset[],
  scrollRef: React.RefObject<HTMLDivElement | null>,
  onContextMenu: () => void,
  dragConfig: DragConfig,
): ItemHandlers {
  const assetsRef = useRef(assets);
  assetsRef.current = assets;
  const menuRef = useRef(onContextMenu);
  menuRef.current = onContextMenu;
  const configRef = useRef(dragConfig);
  configRef.current = dragConfig;

  const onSelect = useCallback((id: number, e: React.MouseEvent) => {
    useSelection.getState().click(
      id,
      assetsRef.current.map((a) => a.id),
      { meta: primary(e), shift: e.shiftKey },
    );
  }, []);

  const onMenu = useCallback((id: number) => {
    const { selected, click } = useSelection.getState();
    if (!selected.has(id)) click(id, assetsRef.current.map((a) => a.id), { meta: false, shift: false });
    menuRef.current();
  }, []);

  // Dragging a selected clip takes the whole selection; an unselected one goes alone.
  const onPointerDown = useCallback((id: number, e: React.PointerEvent) => {
    trackPointer(
      e,
      scrollRef.current,
      () => {
        const { selected, click } = useSelection.getState();
        const order = assetsRef.current.map((a) => a.id);
        const ids = selected.has(id) ? order.filter((x) => selected.has(x)) : [id];
        if (!selected.has(id)) click(id, order, { meta: false, shift: false });
        const thumb = assetsRef.current.find((a) => a.id === id)?.thumb_path;
        return { ids, thumb: thumb ? convertFileSrc(thumb) : null };
      },
      configRef.current,
    );
  }, [scrollRef]);

  return { onSelect, onPointerDown, onMenu };
}

/** Where the "drop here" bar goes on this item, if anywhere. */
function useInsertSide(id: number) {
  return useDnd((s) => (s.target?.kind === "reorder" && s.target.anchorId === id ? s.target.side : null));
}

function useIsDragged(id: number) {
  return useDnd((s) => !!s.drag && s.drag.ids.includes(id));
}

function itemEvents(asset: Asset, h: ItemHandlers) {
  return {
    "data-reorder-id": asset.id,
    onPointerDown: (e: React.PointerEvent) => h.onPointerDown(asset.id, e),
    onClick: (e: React.MouseEvent) => {
      e.stopPropagation();
      h.onSelect(asset.id, e);
    },
    onContextMenu: (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      h.onMenu(asset.id);
    },
  };
}

const Thumb = ({ asset, className }: { asset: Asset; className: string }) =>
  asset.thumb_path ? (
    <img
      src={convertFileSrc(asset.thumb_path)}
      alt=""
      loading="lazy"
      decoding="async"
      draggable={false}
      className={`${className} ${asset.online && !asset.missing ? "" : "opacity-60"}`}
    />
  ) : (
    <div className={`${className} flex items-center justify-center text-[10px] uppercase tracking-wide text-neutral-600`}>
      {asset.kind}
    </div>
  );

const Card = memo(function Card({
  asset,
  selected,
  handlers,
}: {
  asset: Asset;
  selected: boolean;
  handlers: ItemHandlers;
}) {
  const side = useInsertSide(asset.id);
  const dragged = useIsDragged(asset.id);
  return (
    <div
      className={`relative min-w-0 cursor-default ${dragged ? "opacity-40" : ""}`}
      title={asset.rel_path}
      {...itemEvents(asset, handlers)}
    >
      {side && (
        <div
          className={`pointer-events-none absolute inset-y-0 z-10 w-[3px] rounded-full bg-sky-400 ${
            side === "before" ? "-left-[6.5px]" : "-right-[6.5px]"
          }`}
        />
      )}
      <div
        className={`relative aspect-video overflow-hidden rounded bg-neutral-900 ${
          selected ? "ring-2 ring-sky-400" : "ring-1 ring-neutral-800"
        }`}
      >
        <Thumb asset={asset} className="h-full w-full object-contain" />
        {(!asset.online || asset.missing) && (
          <span className="absolute left-1 top-1 rounded bg-black/70 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
            {asset.online ? "Missing" : "Offline"}
          </span>
        )}
        {asset.rating > 0 && (
          <span className="absolute bottom-1 left-1 rounded bg-black/70 px-1 py-0.5 text-[10px] text-amber-300">
            {"★".repeat(asset.rating)}
          </span>
        )}
        {asset.duration_ms != null && (
          <span className="absolute bottom-1 right-1 rounded bg-black/70 px-1 py-0.5 text-[10px] tabular-nums text-neutral-100">
            {formatDuration(asset.duration_ms)}
          </span>
        )}
      </div>
      <div
        className={`truncate px-0.5 pt-1 text-xs ${selected ? "text-sky-300" : "text-neutral-300"}`}
        style={{ height: LABEL_H }}
      >
        {asset.display_name || asset.file_name}
      </div>
    </div>
  );
});

function useWidth(ref: React.RefObject<HTMLDivElement | null>) {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, [ref]);
  return width;
}

export function Grid({ assets, onContextMenu, dragConfig }: ClipViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const width = useWidth(scrollRef);
  const selected = useSelection((s) => s.selected);
  const handlers = useItemHandlers(assets, scrollRef, onContextMenu, dragConfig);

  const inner = Math.max(0, width - PAD * 2);
  const columns = Math.max(1, Math.floor((inner + GAP) / (MIN_CARD + GAP)));
  const cardW = (inner - GAP * (columns - 1)) / columns;
  const rowH = Math.round((cardW * 9) / 16) + LABEL_H + GAP;
  const rowCount = Math.ceil(assets.length / columns);
  gridMetrics.columns = columns;

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowH,
    overscan: 4,
  });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);
  useScrollToFocus(assets, (i) => virtualizer.scrollToIndex(Math.floor(i / columns), { align: "auto" }));

  return (
    <div ref={scrollRef} className="h-full overflow-y-auto" onClick={() => useSelection.getState().clear()}>
      {width > 0 && (
        <div className="relative" style={{ height: virtualizer.getTotalSize() + PAD * 2 }}>
          {virtualizer.getVirtualItems().map((row) => (
            <div
              key={row.key}
              className="absolute grid"
              style={{
                top: row.start + PAD,
                left: PAD,
                right: PAD,
                height: rowH,
                gap: GAP,
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              }}
            >
              {assets.slice(row.index * columns, (row.index + 1) * columns).map((a) => (
                <Card key={a.id} asset={a} selected={selected.has(a.id)} handlers={handlers} />
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

interface Column {
  label: string;
  width: string;
  /** Panel width below which the column is hidden. */
  minWidth: number;
  sort?: SortKey;
  align?: "right";
  render: (a: Asset) => React.ReactNode;
}

const COLUMNS: Column[] = [
  {
    label: "Duration",
    width: "64px",
    minWidth: 0,
    sort: "duration",
    align: "right",
    render: (a) => (a.duration_ms != null ? formatDuration(a.duration_ms) : ""),
  },
  {
    label: "Size",
    width: "72px",
    minWidth: 0,
    sort: "size",
    align: "right",
    render: (a) => formatBytes(a.size_bytes),
  },
  {
    label: "Rating",
    width: "72px",
    minWidth: 560,
    sort: "rating",
    render: (a) => <span className="text-amber-300">{"★".repeat(a.rating)}</span>,
  },
  {
    label: "Captured",
    width: "110px",
    minWidth: 680,
    sort: "captured",
    render: (a) => (a.created_at ? formatDate(a.created_at, false) : ""),
  },
  {
    label: "Resolution",
    width: "96px",
    minWidth: 800,
    render: (a) => (a.width && a.height ? `${a.width} × ${a.height}` : ""),
  },
  {
    label: "Drive",
    width: "110px",
    minWidth: 920,
    render: (a) => (
      <span className={a.online ? "" : "text-amber-300"}>
        {a.volume_label}
        {a.online ? (a.missing ? " · missing" : "") : " · offline"}
      </span>
    ),
  },
];

const Row = memo(function Row({
  asset,
  selected,
  handlers,
  columns,
  template,
}: {
  asset: Asset;
  selected: boolean;
  handlers: ItemHandlers;
  columns: Column[];
  template: string;
}) {
  const side = useInsertSide(asset.id);
  const dragged = useIsDragged(asset.id);
  return (
    <div
      className={`relative grid h-full cursor-default items-center gap-3 rounded px-2 ${
        selected ? "bg-sky-500/20 text-neutral-100" : "text-neutral-300 hover:bg-neutral-900"
      } ${dragged ? "opacity-40" : ""}`}
      style={{ gridTemplateColumns: template }}
      title={asset.rel_path}
      {...itemEvents(asset, handlers)}
    >
      {side && (
        <div
          className={`pointer-events-none absolute inset-x-1 z-10 h-[3px] rounded-full bg-sky-400 ${
            side === "before" ? "-top-[1.5px]" : "-bottom-[1.5px]"
          }`}
        />
      )}
      <div className="relative h-9 overflow-hidden rounded bg-neutral-900">
        <Thumb asset={asset} className="h-full w-full object-contain" />
      </div>
      <div className="min-w-0 truncate">
        {asset.display_name || asset.file_name}
        {asset.display_name && <span className="ml-2 text-xs text-neutral-500">{asset.file_name}</span>}
      </div>
      {columns.map((c) => (
        <div
          key={c.label}
          className={`truncate text-xs tabular-nums text-neutral-400 ${c.align === "right" ? "text-right" : ""}`}
        >
          {c.render(asset)}
        </div>
      ))}
    </div>
  );
});

export function ClipList({
  assets,
  onContextMenu,
  dragConfig,
  sort,
  onSort,
}: ClipViewProps & {
  sort: { key: SortKey; desc: boolean };
  /** Clicking a header sorts by it; clicking the current one flips the direction. */
  onSort: (key: SortKey) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const width = useWidth(scrollRef);
  const selected = useSelection((s) => s.selected);
  const handlers = useItemHandlers(assets, scrollRef, onContextMenu, dragConfig);
  const columns = COLUMNS.filter((c) => width >= c.minWidth);
  const template = `64px minmax(0,1fr) ${columns.map((c) => c.width).join(" ")}`;

  const virtualizer = useVirtualizer({
    count: assets.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 8,
  });
  useScrollToFocus(assets, (i) => virtualizer.scrollToIndex(i, { align: "auto" }));

  const header = (label: string, key: SortKey | undefined, align?: "right") => (
    <button
      key={label}
      disabled={!key}
      onClick={() => key && onSort(key)}
      className={`truncate text-[11px] font-semibold uppercase tracking-wide ${
        key && sort.key === key ? "text-neutral-200" : "text-neutral-500"
      } ${key ? "hover:text-neutral-200" : "cursor-default"} ${align === "right" ? "text-right" : "text-left"}`}
    >
      {label}
      {key && sort.key === key && (sort.desc ? " ↓" : " ↑")}
    </button>
  );

  return (
    <div className="flex h-full flex-col">
      <div
        className="grid shrink-0 items-center gap-3 border-b border-neutral-800 px-5 py-1.5"
        style={{ gridTemplateColumns: template }}
      >
        <span />
        {header("Name", "name")}
        {columns.map((c) => header(c.label, c.sort, c.align))}
      </div>
      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-1"
        onClick={() => useSelection.getState().clear()}
      >
        <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const a = assets[item.index];
            return (
              <div
                key={a.id}
                className="absolute inset-x-0 py-px"
                style={{ top: item.start, height: ROW_H }}
              >
                <Row
                  asset={a}
                  selected={selected.has(a.id)}
                  handlers={handlers}
                  columns={columns}
                  template={template}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
