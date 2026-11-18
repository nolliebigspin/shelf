import { create } from "zustand";
import type { SidebarList } from "./api";
import { suppressNextClick } from "./dnd";

/**
 * Drag to sort albums, tags and drives in the sidebar. Items mark themselves with
 * `data-sort-list` / `data-sort-id`; a drag only ever moves within its own list.
 */

interface SortState {
  list: SidebarList | null;
  dragId: number | null;
  /** Where it would land: before or after `overId`. */
  overId: number | null;
  side: "before" | "after" | null;
}

export const useSidebarSort = create<SortState>(() => ({ list: null, dragId: null, overId: null, side: null }));

const THRESHOLD = 4;

/** `ids` with `id` moved before / after `overId`. */
export function moveInList(ids: number[], id: number, overId: number, side: "before" | "after"): number[] {
  const rest = ids.filter((x) => x !== id);
  const at = rest.indexOf(overId);
  if (at === -1) return ids;
  rest.splice(side === "before" ? at : at + 1, 0, id);
  return rest;
}

/** Call from an item's pointerdown; `onDrop` gets the new order if it changed. */
export function trackSidebarSort(
  e: React.PointerEvent,
  list: SidebarList,
  id: number,
  order: () => number[],
  onDrop: (ids: number[]) => void,
) {
  if (e.button !== 0) return;
  const pointerId = e.pointerId;
  const startX = e.clientX;
  const startY = e.clientY;
  let active = false;

  const hit = (x: number, y: number) => {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>(`[data-sort-list="${list}"]`);
    const overId = el ? Number(el.dataset.sortId) : NaN;
    if (!el || Number.isNaN(overId) || overId === id) return { overId: null, side: null };
    const r = el.getBoundingClientRect();
    return { overId, side: y < r.top + r.height / 2 ? ("before" as const) : ("after" as const) };
  };

  const cleanup = () => {
    window.removeEventListener("pointermove", onMove, true);
    window.removeEventListener("pointerup", onUp, true);
    window.removeEventListener("pointercancel", cleanup, true);
    window.removeEventListener("keydown", onKey, true);
    try {
      document.body.releasePointerCapture(pointerId);
    } catch {
      // Already released.
    }
    document.body.style.cursor = "";
    useSidebarSort.setState({ list: null, dragId: null, overId: null, side: null });
  };

  const onMove = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    if (!active) {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < THRESHOLD) return;
      active = true;
      try {
        document.body.setPointerCapture(pointerId);
      } catch {
        // Window listeners still see moves inside the window.
      }
      document.body.style.cursor = "grabbing";
      useSidebarSort.setState({ list, dragId: id });
    }
    ev.preventDefault();
    const { overId, side } = hit(ev.clientX, ev.clientY);
    const s = useSidebarSort.getState();
    if (s.overId !== overId || s.side !== side) useSidebarSort.setState({ overId, side });
  };

  const onUp = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    const wasActive = active;
    const { overId, side } = hit(ev.clientX, ev.clientY);
    cleanup();
    if (!wasActive) return;
    suppressNextClick();
    if (overId === null || side === null) return;
    const before = order();
    const after = moveInList(before, id, overId, side);
    if (after.join(",") !== before.join(",")) onDrop(after);
  };

  const onKey = (ev: KeyboardEvent) => {
    if (ev.key === "Escape" && active) {
      ev.stopPropagation();
      cleanup();
    }
  };

  window.addEventListener("pointermove", onMove, true);
  window.addEventListener("pointerup", onUp, true);
  window.addEventListener("pointercancel", cleanup, true);
  window.addEventListener("keydown", onKey, true);
}
