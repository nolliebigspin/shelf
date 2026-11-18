import { create } from "zustand";

/**
 * In-app drag and drop for clips.
 *
 * Everything inside the window is plain pointer events, hit-tested with the
 * webview's own coordinates (`elementFromPoint(clientX, clientY)`), so the drop
 * lands exactly under the cursor. Only when the pointer leaves the window does
 * the app hand over to a native macOS file drag (for Final Cut, Finder, …).
 *
 * Drop targets mark themselves in the DOM:
 * - `data-drop="album:<id>"` / `data-drop="trash"` on sidebar items
 * - `data-reorder-id="<asset id>"` on grid cards and list rows
 */

export type DropTarget =
  | { kind: "album"; id: number }
  | { kind: "trash" }
  /** Insert before `beforeId` (null = at the end); drawn as a bar on `anchorId`'s `side`. */
  | { kind: "reorder"; beforeId: number | null; anchorId: number; side: "before" | "after" }
  /** Over the clips while the current sort can't be rearranged by hand. */
  | { kind: "no-reorder" };

interface DragSession {
  ids: number[];
  /** Thumbnail URL of the clip under the pointer, for the ghost. */
  thumb: string | null;
  /** What dropping at the current spot will do, shown next to the cursor. */
  label: string | null;
}

interface DndState {
  drag: DragSession | null;
  target: DropTarget | null;
}

export const useDnd = create<DndState>(() => ({ drag: null, target: null }));

export interface DragConfig {
  /** Ids in display order, for working out where a reorder lands. */
  order: () => number[];
  layout: "grid" | "list";
  /** Whether the current view and sort allow rearranging by hand. */
  canReorder: () => boolean;
  /** Whether album / Trash targets accept drops right now. */
  canDropOnSidebar: () => boolean;
  describe: (target: DropTarget | null, ids: number[]) => string | null;
  onDrop: (ids: number[], target: DropTarget) => void;
  /** The pointer left the window: hand over to a native file drag. */
  onLeaveWindow: (ids: number[]) => void;
}

const THRESHOLD = 4; // px before a press becomes a drag
const EDGE = 48; // px from the scroll area's edge where auto-scroll kicks in
const MAX_SPEED = 18; // px per frame

let ghostEl: HTMLElement | null = null;
/** The floating preview registers itself so it can follow the pointer without re-rendering React. */
export function registerGhost(el: HTMLElement | null) {
  ghostEl = el;
}

function sameTarget(a: DropTarget | null, b: DropTarget | null) {
  return JSON.stringify(a) === JSON.stringify(b);
}

function hitTest(x: number, y: number, ids: number[], config: DragConfig): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  if (!el) return null;
  const drop = el.closest<HTMLElement>("[data-drop]")?.dataset.drop;
  if (drop) {
    if (!config.canDropOnSidebar()) return null;
    if (drop === "trash") return { kind: "trash" };
    if (drop.startsWith("album:")) return { kind: "album", id: Number(drop.slice("album:".length)) };
    return null;
  }
  const item = el.closest<HTMLElement>("[data-reorder-id]");
  if (!item) return null;
  if (!config.canReorder()) return { kind: "no-reorder" };
  const anchorId = Number(item.dataset.reorderId);
  const dragged = new Set(ids);
  if (dragged.has(anchorId)) return null;
  const r = item.getBoundingClientRect();
  const side =
    config.layout === "grid"
      ? x < r.left + r.width / 2
        ? "before"
        : "after"
      : y < r.top + r.height / 2
        ? "before"
        : "after";
  let beforeId: number | null = anchorId;
  if (side === "after") {
    const order = config.order();
    const i = order.indexOf(anchorId);
    beforeId = order.slice(i + 1).find((id) => !dragged.has(id)) ?? null;
  }
  return { kind: "reorder", beforeId, anchorId, side };
}

/** Swallow the click that follows a drag so it doesn't change the selection. */
function suppressNextClick() {
  const stop = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener("click", stop, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", stop, { capture: true }), 0);
}

/**
 * Call from an item's pointerdown. Nothing happens until the pointer moves a few
 * pixels; then `start()` decides which clips are dragged (e.g. the selection).
 */
export function trackPointer(
  e: React.PointerEvent,
  scrollEl: HTMLElement | null,
  start: () => { ids: number[]; thumb: string | null } | null,
  config: DragConfig,
) {
  if (e.button !== 0) return;
  const pointerId = e.pointerId;
  const startX = e.clientX;
  const startY = e.clientY;
  let x = startX;
  let y = startY;
  let session: { ids: number[] } | null = null;
  let frame = 0;

  const moveGhost = () => {
    if (ghostEl) ghostEl.style.transform = `translate(${x + 14}px, ${y + 16}px)`;
  };

  const retarget = () => {
    if (!session) return;
    const target = hitTest(x, y, session.ids, config);
    const state = useDnd.getState();
    if (!sameTarget(state.target, target)) {
      useDnd.setState({
        target,
        drag: state.drag && { ...state.drag, label: config.describe(target, session.ids) },
      });
    }
  };

  // Scroll the clip area while the pointer rests near its top or bottom edge.
  const autoScroll = () => {
    frame = 0;
    if (!session || !scrollEl) return;
    const r = scrollEl.getBoundingClientRect();
    let dy = 0;
    if (x >= r.left && x <= r.right) {
      if (y < r.top + EDGE) dy = -Math.ceil(((r.top + EDGE - y) / EDGE) * MAX_SPEED);
      else if (y > r.bottom - EDGE) dy = Math.ceil(((y - (r.bottom - EDGE)) / EDGE) * MAX_SPEED);
    }
    if (dy !== 0) {
      scrollEl.scrollTop += dy;
      retarget();
    }
    frame = requestAnimationFrame(autoScroll);
  };

  const cleanup = () => {
    window.removeEventListener("pointermove", onMove, true);
    window.removeEventListener("pointerup", onUp, true);
    window.removeEventListener("pointercancel", onCancel, true);
    window.removeEventListener("keydown", onKey, true);
    if (frame) cancelAnimationFrame(frame);
    try {
      document.body.releasePointerCapture(pointerId);
    } catch {
      // Already released.
    }
    document.body.style.cursor = "";
    useDnd.setState({ drag: null, target: null });
  };

  const onMove = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    x = ev.clientX;
    y = ev.clientY;
    if (!session) {
      if (Math.hypot(x - startX, y - startY) < THRESHOLD) return;
      const begun = start();
      if (!begun || begun.ids.length === 0) {
        cleanup();
        return;
      }
      session = { ids: begun.ids };
      // Keep getting moves even over other elements or outside the window.
      try {
        document.body.setPointerCapture(pointerId);
      } catch {
        // Not capturable; window listeners still see moves inside the window.
      }
      document.body.style.cursor = "grabbing";
      useDnd.setState({ drag: { ...begun, label: config.describe(null, begun.ids) }, target: null });
      frame = requestAnimationFrame(autoScroll);
    }
    ev.preventDefault();
    moveGhost();
    if (x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight) {
      const ids = session.ids;
      cleanup();
      suppressNextClick();
      config.onLeaveWindow(ids);
      return;
    }
    retarget();
  };

  const onUp = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    if (!session) {
      cleanup();
      return;
    }
    x = ev.clientX;
    y = ev.clientY;
    const target = hitTest(x, y, session.ids, config);
    const ids = session.ids;
    cleanup();
    suppressNextClick();
    if (target && target.kind !== "no-reorder") config.onDrop(ids, target);
  };

  const onCancel = (ev: PointerEvent) => {
    if (ev.pointerId === pointerId) cleanup();
  };

  const onKey = (ev: KeyboardEvent) => {
    if (ev.key === "Escape" && session) {
      ev.stopPropagation();
      cleanup();
    }
  };

  window.addEventListener("pointermove", onMove, true);
  window.addEventListener("pointerup", onUp, true);
  window.addEventListener("pointercancel", onCancel, true);
  window.addEventListener("keydown", onKey, true);
}
