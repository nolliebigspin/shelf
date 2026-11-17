import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { convertFileSrc } from "@tauri-apps/api/core";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import type { Asset } from "./api";
import { useSelection } from "./store";

const MIN_CARD = 190; // px, cards stretch to fill the row
const GAP = 10;
const PAD = 12;
const LABEL_H = 26;

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

const Card = memo(function Card({
  asset,
  selected,
  onSelect,
  onDrag,
}: {
  asset: Asset;
  selected: boolean;
  onSelect: (id: number, e: React.MouseEvent) => void;
  onDrag: (id: number) => void;
}) {
  return (
    <div
      className="min-w-0 cursor-default"
      title={asset.rel_path}
      draggable
      onDragStart={(e) => {
        // Replace the webview's own drag with a native file drag.
        e.preventDefault();
        onDrag(asset.id);
      }}
      onClick={(e) => {
        e.stopPropagation();
        onSelect(asset.id, e);
      }}
    >
      <div
        className={`relative aspect-video overflow-hidden rounded bg-neutral-900 ${
          selected ? "ring-2 ring-sky-400" : "ring-1 ring-neutral-800"
        }`}
      >
        {asset.thumb_path ? (
          <img
            src={convertFileSrc(asset.thumb_path)}
            alt=""
            loading="lazy"
            decoding="async"
            draggable={false}
            className={`h-full w-full object-contain ${asset.online && !asset.missing ? "" : "opacity-60"}`}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-[11px] uppercase tracking-wide text-neutral-600">
            {asset.kind}
          </div>
        )}
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

export function Grid({ assets }: { assets: Asset[] }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const selected = useSelection((s) => s.selected);
  const assetsRef = useRef(assets);
  assetsRef.current = assets;
  const onSelect = useCallback((id: number, e: React.MouseEvent) => {
    useSelection.getState().click(
      id,
      assetsRef.current.map((a) => a.id),
      { meta: e.metaKey, shift: e.shiftKey },
    );
  }, []);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  // Drag the original files (never copies) out to Final Cut, Premiere, Resolve or Finder.
  // Dragging a selected clip takes the whole selection; an unselected one goes alone.
  const onDrag = useCallback((id: number) => {
    const { selected, click } = useSelection.getState();
    const all = assetsRef.current;
    if (!selected.has(id)) click(id, all.map((a) => a.id), { meta: false, shift: false });
    const ids = selected.has(id) ? selected : new Set([id]);
    const dragged = all.filter((a) => ids.has(a.id) && a.online && !a.missing && a.path);
    const icon = all.find((a) => a.id === id)?.thumb_path ?? dragged.find((a) => a.thumb_path)?.thumb_path;
    if (dragged.length === 0 || !icon) return;
    // "copy" so a drop onto Finder can never move the original off the drive.
    startDrag({ item: dragged.map((a) => a.path!), icon, mode: "copy" }).catch(() => {});
  }, []);

  const inner = Math.max(0, width - PAD * 2);
  const columns = Math.max(1, Math.floor((inner + GAP) / (MIN_CARD + GAP)));
  const cardW = (inner - GAP * (columns - 1)) / columns;
  const rowH = Math.round((cardW * 9) / 16) + LABEL_H + GAP;
  const rowCount = Math.ceil(assets.length / columns);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowH,
    overscan: 4,
  });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);

  return (
    <div
      ref={scrollRef}
      className="h-full overflow-y-auto"
      onClick={() => useSelection.getState().clear()}
    >
      {width > 0 && (
        <div
          className="relative"
          style={{ height: virtualizer.getTotalSize() + PAD * 2 }}
        >
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
                <Card key={a.id} asset={a} selected={selected.has(a.id)}
                  onSelect={onSelect}
                  onDrag={onDrag}
                />
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
