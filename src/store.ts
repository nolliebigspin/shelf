import { create } from "zustand";

interface SelectionState {
  selected: Set<number>;
  /** Where a Shift-click range starts. */
  anchor: number | null;
  /** `order` is the ids currently in the grid, in display order. */
  click: (id: number, order: number[], mods: { meta: boolean; shift: boolean }) => void;
  selectAll: (order: number[]) => void;
  clear: () => void;
  /** Drop selected ids that are no longer in the grid. */
  retain: (order: number[]) => void;
}

export const useSelection = create<SelectionState>((set, get) => ({
  selected: new Set(),
  anchor: null,
  click: (id, order, { meta, shift }) => {
    const { selected, anchor } = get();
    if (shift && anchor !== null) {
      const a = order.indexOf(anchor);
      const b = order.indexOf(id);
      if (a !== -1 && b !== -1) {
        const range = order.slice(Math.min(a, b), Math.max(a, b) + 1);
        set({ selected: new Set(meta ? [...selected, ...range] : range) });
        return;
      }
    }
    if (meta) {
      const next = new Set(selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      set({ selected: next, anchor: id });
      return;
    }
    set({ selected: new Set([id]), anchor: id });
  },
  selectAll: (order) => set({ selected: new Set(order), anchor: order[0] ?? null }),
  clear: () => set({ selected: new Set(), anchor: null }),
  retain: (order) => {
    const { selected, anchor } = get();
    if (selected.size === 0) return;
    const present = new Set(order);
    const next = new Set([...selected].filter((id) => present.has(id)));
    if (next.size !== selected.size) {
      set({ selected: next, anchor: anchor !== null && present.has(anchor) ? anchor : null });
    }
  },
}));
