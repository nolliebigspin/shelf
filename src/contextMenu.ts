import { Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import type { Album, Asset } from "./api";

interface ClipMenuActions {
  reveal: (id: number) => void;
  openExternal: (id: number) => void;
  addToAlbum: (albumId: number) => void;
  /** Only set while an album is open. */
  removeFromAlbum?: () => void;
  trash: () => void;
  restore: () => void;
  deleteFromDisk: () => void;
}

/** Native right-click menu for the selected clips. */
export async function showClipMenu({
  selection,
  albums,
  inTrash,
  albumName,
  actions,
}: {
  selection: Asset[];
  albums: Album[];
  inTrash: boolean;
  albumName: string | null;
  actions: ClipMenuActions;
}) {
  if (selection.length === 0) return;
  const single = selection.length === 1 ? selection[0] : null;
  const available = !!single && single.online && !single.missing;
  const count = selection.length > 1 ? ` ${selection.length.toLocaleString()} Clips` : "";
  const separator = () => PredefinedMenuItem.new({ item: "Separator" });

  const items = inTrash
    ? [
        await MenuItem.new({ text: `Put Back${count}`, action: actions.restore }),
        await separator(),
        await MenuItem.new({ text: `Delete${count} from Drive…`, action: actions.deleteFromDisk }),
      ]
    : [
        await MenuItem.new({
          text: "Reveal in Finder",
          enabled: available,
          action: () => single && actions.reveal(single.id),
        }),
        await MenuItem.new({
          text: "Open Externally",
          enabled: available,
          action: () => single && actions.openExternal(single.id),
        }),
        await separator(),
        await Submenu.new({
          text: "Add to Album",
          enabled: albums.length > 0,
          items: await Promise.all(
            albums.map((a) => MenuItem.new({ text: a.name, action: () => actions.addToAlbum(a.id) })),
          ),
        }),
        ...(actions.removeFromAlbum && albumName !== null
          ? [await MenuItem.new({ text: `Remove from “${albumName}”`, action: actions.removeFromAlbum })]
          : []),
        await separator(),
        await MenuItem.new({ text: `Move${count} to Trash`, action: actions.trash }),
      ];

  const menu = await Menu.new({ items });
  await menu.popup();
}
