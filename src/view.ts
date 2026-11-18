import type { SortKey } from "./api";

/** What the clip area is showing. */
export type View =
  | { kind: "all" }
  | { kind: "trash" }
  | { kind: "volume"; id: number }
  | { kind: "album"; id: number }
  | { kind: "tag"; name: string };

export type Sort = { key: SortKey; desc: boolean };
export type Layout = "grid" | "list";

export const SORT_LABELS: Record<SortKey, string> = {
  custom: "Custom",
  name: "Name",
  captured: "Date captured",
  added: "Date added",
  size: "Size",
  duration: "Duration",
  rating: "Rating",
  kind: "Kind",
  trashed: "Date trashed",
};

/** Albums, drives and All clips can be arranged by hand. */
const hasCustomOrder = (v: View) => v.kind === "all" || v.kind === "volume" || v.kind === "album";
export const viewKey = (v: View) =>
  v.kind === "volume" || v.kind === "album" ? `${v.kind}:${v.id}` : v.kind === "tag" ? "tag" : v.kind;
export const sortKeysFor = (v: View): SortKey[] =>
  (Object.keys(SORT_LABELS) as SortKey[]).filter(
    (k) => (k !== "custom" || hasCustomOrder(v)) && (k !== "trashed" || v.kind === "trash"),
  );
export const defaultSort = (v: View): Sort =>
  v.kind === "trash"
    ? { key: "trashed", desc: true }
    : hasCustomOrder(v)
      ? { key: "custom", desc: false }
      : { key: "name", desc: false };
/** Dates, sizes and ratings read best biggest/newest first. */
export const DESC_FIRST: SortKey[] = ["captured", "added", "size", "duration", "rating", "trashed"];

export function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}
export function writeJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Not remembered; harmless.
  }
}
