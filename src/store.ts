import { create } from "zustand";

interface SelectionState {
  selected: Set<number>;
  /** Where a Shift-click / Shift-arrow range starts. */
  anchor: number | null;
  /** The clip the arrow keys move from (the moving end of a Shift range). */
  cursor: number | null;
  /** Set by keyboard moves so the grid / list scrolls that clip into view. */
  focus: { id: number } | null;
  /** `order` is the ids currently in the grid, in display order. */
  click: (id: number, order: number[], mods: { meta: boolean; shift: boolean }) => void;
  /**
   * Move the cursor by `delta` places in `order` (±Infinity jumps to the ends).
   * With `extend`, the selection grows from the anchor like Shift-arrow in Finder.
   */
  step: (order: number[], delta: number, extend: boolean) => void;
  selectAll: (order: number[]) => void;
  clear: () => void;
  /** Drop selected ids that are no longer in the grid. */
  retain: (order: number[]) => void;
}

export const useSelection = create<SelectionState>((set, get) => ({
  selected: new Set(),
  anchor: null,
  cursor: null,
  focus: null,
  click: (id, order, { meta, shift }) => {
    const { selected, anchor } = get();
    if (shift && anchor !== null) {
      const a = order.indexOf(anchor);
      const b = order.indexOf(id);
      if (a !== -1 && b !== -1) {
        const range = order.slice(Math.min(a, b), Math.max(a, b) + 1);
        set({ selected: new Set(meta ? [...selected, ...range] : range), cursor: id });
        return;
      }
    }
    if (meta) {
      const next = new Set(selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      set({ selected: next, anchor: id, cursor: id });
      return;
    }
    set({ selected: new Set([id]), anchor: id, cursor: id });
  },
  step: (order, delta, extend) => {
    if (order.length === 0) return;
    const { cursor, anchor, selected } = get();
    const from = cursor !== null && order.includes(cursor) ? order.indexOf(cursor) : -1;
    let to: number;
    if (from === -1 || selected.size === 0) to = delta < 0 ? order.length - 1 : 0;
    else to = Math.max(0, Math.min(order.length - 1, from + (Number.isFinite(delta) ? delta : delta * order.length)));
    const id = order[to];
    if (extend && anchor !== null && order.includes(anchor) && selected.size > 0) {
      const a = order.indexOf(anchor);
      set({ selected: new Set(order.slice(Math.min(a, to), Math.max(a, to) + 1)), cursor: id, focus: { id } });
    } else {
      set({ selected: new Set([id]), anchor: id, cursor: id, focus: { id } });
    }
  },
  selectAll: (order) => set({ selected: new Set(order), anchor: order[0] ?? null, cursor: order[0] ?? null }),
  clear: () => set({ selected: new Set(), anchor: null, cursor: null }),
  retain: (order) => {
    const { selected, anchor, cursor } = get();
    if (selected.size === 0) return;
    const present = new Set(order);
    const next = new Set([...selected].filter((id) => present.has(id)));
    if (next.size !== selected.size) {
      set({
        selected: next,
        anchor: anchor !== null && present.has(anchor) ? anchor : null,
        cursor: cursor !== null && present.has(cursor) ? cursor : null,
      });
    }
  },
}));
