import { invoke } from "@tauri-apps/api/core";

export interface AppStatus {
  db_path: string;
  thumbs_dir: string;
  ffmpeg: string | null;
  ffprobe: string | null;
  ffmpeg_error: string | null;
}

export const appStatus = () => invoke<AppStatus>("app_status");

export interface Volume {
  id: number;
  uuid: string;
  label: string;
  mount_path: string | null;
  online: boolean;
  asset_count: number;
}

export interface ScanProgress {
  volume_id: number;
  found: number;
  processed: number;
  total: number;
  done: boolean;
  error: string | null;
}

export const addRoot = (path: string) => invoke<number>("add_root", { path });
export const refreshVolumes = () => invoke<Volume[]>("refresh_volumes");

export interface Asset {
  id: number;
  volume_id: number;
  rel_path: string;
  file_name: string;
  kind: "video" | "image";
  size_bytes: number;
  duration_ms: number | null;
  width: number | null;
  height: number | null;
  codec: string | null;
  created_at: number | null;
  thumb_path: string | null;
  display_name: string | null;
  rating: number;
  missing: boolean;
  /** Set while the clip is in Shelf's Trash. */
  trashed_at: number | null;
  online: boolean;
  path: string | null;
  volume_label: string;
  tags: string[];
}

export interface AssetFilter {
  volume_id?: number | null;
  album_id?: number | null;
  search?: string | null;
  tag?: string | null;
  min_rating?: number | null;
  /** true lists only the Trash; otherwise trashed clips are left out. */
  trashed?: boolean | null;
  sort?: SortKey | null;
  descending?: boolean | null;
}

export type SortKey =
  | "custom"
  | "name"
  | "captured"
  | "added"
  | "size"
  | "duration"
  | "rating"
  | "kind"
  | "trashed";

/** Which hand-arranged order a move applies to: an album, a drive, or All clips (both null). */
export interface OrderScope {
  album_id: number | null;
  volume_id: number | null;
}

export interface Tag {
  id: number;
  name: string;
  asset_count: number;
}

export interface Album {
  id: number;
  name: string;
  created_at: number | null;
  asset_count: number;
}

export const listAssets = (filter: AssetFilter = {}) => invoke<Asset[]>("list_assets", { filter });

export const updateAsset = (id: number, patch: { display_name?: string; rating?: number }) =>
  invoke<void>("update_asset", { id, patch });
export const setTags = (assetIds: number[], names: string[], mode: "add" | "remove") =>
  invoke<void>("set_tags", { assetIds, names, mode });
export const listTags = () => invoke<Tag[]>("list_tags");
export const listAlbums = () => invoke<Album[]>("list_albums");
export const createAlbum = (name: string) => invoke<number>("create_album", { name });
export const addToAlbum = (albumId: number, assetIds: number[]) =>
  invoke<void>("add_to_album", { albumId, assetIds });
export const removeFromAlbum = (albumId: number, assetIds: number[]) =>
  invoke<void>("remove_from_album", { albumId, assetIds });
export const reveal = (assetId: number) => invoke<void>("reveal", { assetId });
export const openExternal = (assetId: number) => invoke<void>("open_external", { assetId });
export const rescan = (volumeId: number) => invoke<void>("rescan", { volumeId });

export interface DeleteReport {
  deleted: number;
  skipped_offline: number;
  failed: string[];
}

export const trashAssets = (assetIds: number[]) => invoke<void>("trash_assets", { assetIds });
export const restoreAssets = (assetIds: number[]) => invoke<void>("restore_assets", { assetIds });
export const trashCount = () => invoke<number>("trash_count");
/** Permanently deletes the files of trashed clips from their drives. */
export const deleteFromDisk = (assetIds: number[]) => invoke<DeleteReport>("delete_from_disk", { assetIds });
/** Small translucent PNG (data URL) used as the drag image. */
export const dragIcon = (assetIds: number[]) => invoke<string>("drag_icon", { assetIds });
/** Rearrange by hand: `assetIds` go right before `beforeId` (null = at the end). */
export const moveAssets = (scope: OrderScope, assetIds: number[], beforeId: number | null) =>
  invoke<void>("move_assets", { scope, assetIds, beforeId });
export const renameAlbum = (albumId: number, name: string) => invoke<void>("rename_album", { albumId, name });
/** Deletes the album only; its clips stay in the catalog. */
export const deleteAlbum = (albumId: number) => invoke<void>("delete_album", { albumId });
/** Renaming onto an existing tag merges the two. */
export const renameTag = (name: string, newName: string) => invoke<void>("rename_tag", { name, newName });
/** Removes the tag from every clip. */
export const deleteTag = (name: string) => invoke<void>("delete_tag", { name });

/** The sidebar lists you can sort by dragging. */
export type SidebarList = "albums" | "tags" | "volumes";
/** Save a hand-arranged order (`ids` top to bottom). */
export const reorderSidebar = (list: SidebarList, ids: number[]) => invoke<void>("reorder_sidebar", { list, ids });
