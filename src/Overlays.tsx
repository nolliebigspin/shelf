import { registerGhost, useDnd } from "./dnd";
import { mod, revealLabel, trashKey } from "./platform";

const SHORTCUTS: [string, string][] = [
  ["← → ↑ ↓", "Move between clips"],
  ["⇧ + arrows", "Extend the selection"],
  [`${mod}↑ / ${mod}↓, Home / End`, "First / last clip"],
  [`${mod}A`, "Select all"],
  ["Esc", "Deselect (or cancel a drag)"],
  ["Space", "Play / pause"],
  ["Return", "Rename the selected clip"],
  ["1 – 5, 0", "Rate, clear rating"],
  [`${mod}O`, "Open externally"],
  [`${mod}R`, revealLabel],
  [trashKey, "Move to Trash (in the Trash: Put Back)"],
  [`${mod}F`, "Search"],
  [`${mod}1 / ${mod}2`, "Grid / list layout"],
  ["?", "Show these shortcuts"],
];

export function ShortcutsHelp({ onClose }: { onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        role="dialog"
        aria-label="Keyboard shortcuts"
        className="w-[26rem] max-w-[calc(100vw-2rem)] rounded-lg border border-neutral-700 bg-neutral-900 p-4 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center">
          <h2 className="font-semibold text-neutral-100">Keyboard shortcuts</h2>
          <button onClick={onClose} className="ml-auto rounded px-2 text-neutral-500 hover:text-neutral-100" aria-label="Close">
            ×
          </button>
        </div>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
          {SHORTCUTS.map(([keys, what]) => (
            <div key={keys} className="contents">
              <dt className="whitespace-nowrap font-mono text-xs text-neutral-200">{keys}</dt>
              <dd className="text-neutral-400">{what}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}

/** The floating preview next to the cursor while clips are dragged inside the window. */
export function DragGhost() {
  const drag = useDnd((s) => s.drag);
  const blocked = useDnd((s) => s.target?.kind === "no-reorder");
  return (
    <div ref={registerGhost} className="pointer-events-none fixed left-0 top-0 z-50">
      {drag && (
        <div className="flex flex-col items-start gap-1">
          <div className="relative">
            {drag.ids.length > 1 && (
              <div className="absolute left-1.5 top-1.5 h-10 w-[72px] rounded bg-neutral-700 ring-1 ring-neutral-500" />
            )}
            <div className="relative h-10 w-[72px] overflow-hidden rounded bg-neutral-900 opacity-90 shadow-lg ring-1 ring-sky-400">
              {drag.thumb && <img src={drag.thumb} alt="" className="h-full w-full object-cover" />}
            </div>
            {drag.ids.length > 1 && (
              <span className="absolute -right-2.5 -top-2 rounded-full bg-sky-500 px-1.5 text-[11px] font-semibold tabular-nums text-white shadow">
                {drag.ids.length}
              </span>
            )}
          </div>
          {drag.label && (
            <span
              className={`whitespace-nowrap rounded px-1.5 py-0.5 text-[11px] shadow ${
                blocked ? "bg-neutral-800 text-amber-300" : "bg-sky-500 text-white"
              }`}
            >
              {drag.label}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
