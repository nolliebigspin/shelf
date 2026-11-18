import { useEffect, useRef, useState } from "react";

function readStored(key: string): number | null {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

/** A panel width remembered across launches (per viewer, best effort). */
export function usePanelWidth(key: string, initial: number) {
  const [width, setWidth] = useState(() => readStored(key) ?? initial);
  useEffect(() => {
    try {
      localStorage.setItem(key, String(Math.round(width)));
    } catch {
      // Storage unavailable: the width just isn't remembered.
    }
  }, [key, width]);
  return [width, setWidth] as const;
}

export function useWindowWidth() {
  const [w, setW] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => setW(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return w;
}

export const clamp = (v: number, min: number, max: number) => Math.max(min, Math.min(v, Math.max(min, max)));

/**
 * Drag handle on the border between a side panel and the grid. `side` is where
 * the panel sits, so dragging toward the grid always makes it wider.
 * Double-click resets to the default width.
 */
export function ResizeHandle({
  side,
  width,
  min,
  max,
  onChange,
  onReset,
}: {
  side: "left" | "right";
  width: number;
  min: number;
  max: number;
  onChange: (w: number) => void;
  onReset: () => void;
}) {
  const start = useRef<{ x: number; w: number } | null>(null);
  const [active, setActive] = useState(false);
  return (
    <div className="relative z-10 w-0 shrink-0">
      <div
        role="separator"
        aria-orientation="vertical"
        title="Drag to resize · double-click to reset"
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          start.current = { x: e.clientX, w: width };
          setActive(true);
          document.body.style.cursor = "col-resize";
        }}
        onPointerMove={(e) => {
          if (!start.current) return;
          const dx = e.clientX - start.current.x;
          onChange(clamp(start.current.w + (side === "left" ? dx : -dx), min, max));
        }}
        onPointerUp={(e) => {
          start.current = null;
          setActive(false);
          document.body.style.cursor = "";
          e.currentTarget.releasePointerCapture(e.pointerId);
        }}
        onDoubleClick={onReset}
        className="group absolute inset-y-0 -left-[3px] w-[6px] cursor-col-resize"
      >
        <div
          className={`absolute inset-y-0 left-[2.5px] w-px transition-colors ${
            active ? "bg-sky-400" : "bg-transparent group-hover:bg-sky-400/70"
          }`}
        />
      </div>
    </div>
  );
}
