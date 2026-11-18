/** Platform conventions for shortcuts and wording (macOS, Windows, Linux). */
const ua = navigator.userAgent;
export const isMac = /Mac/i.test(ua);
const isWindows = /Windows/i.test(ua);

/** The shortcut modifier: ⌘ on macOS, Ctrl elsewhere. */
export const primary = (e: { metaKey: boolean; ctrlKey: boolean }) => (isMac ? e.metaKey : e.ctrlKey);
/** How the modifier is written in labels: "⌘O" vs "Ctrl+O". */
export const mod = isMac ? "⌘" : "Ctrl+";
/** The Move to Trash key as written in labels. */
export const trashKey = isMac ? "⌘⌫" : "Delete";

export const fileManager = isMac ? "Finder" : isWindows ? "Explorer" : "the file manager";
export const revealLabel = isMac ? "Reveal in Finder" : isWindows ? "Show in Explorer" : "Show in Folder";
/** Editors clips can be dragged into (Final Cut is macOS-only). */
export const editors = isMac ? "Final Cut, Premiere, Resolve" : "Premiere, Resolve";
export const systemTrash = isMac ? "the Finder Trash" : isWindows ? "the Recycle Bin" : "the system Trash";
