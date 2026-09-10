import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/* Ponte tipada pros comandos Rust (crates/pc-app/src/commands.rs) e eventos
   do scan (src/scan.rs). Os tipos espelham os DTOs de src/dto.rs. */

export type Source =
  | { kind: "library" }
  | { kind: "playlist"; id: string }
  | { kind: "artist"; id: number };

export type Sort = "artist-album" | "title" | "recently-added";

export interface Stats {
  tracks: number;
  albums: number;
  artists: number;
}

export interface TrackRow {
  id: number;
  title: string;
  artist: string | null;
  artistId: number | null;
  album: string | null;
  trackNo: number | null;
  durationMs: number | null;
  art: string | null;
}

export interface Playlist {
  id: string;
  name: string;
  items: number;
  covers: string[];
  linked: boolean;
  hasImage: boolean;
}

export interface Artist {
  id: number;
  name: string;
  tracks: number;
  cover: string | null;
  hasImage: boolean;
}

export interface OpenResult {
  total: number;
  kind: "library" | "playlist" | "artist";
  title: string;
  subtitle: string;
  heroArt: string | null;
}

export interface ScanProgress {
  done: number;
}
export interface ScanDone {
  added: number;
  updated: number;
  removed: number;
  unchanged: number;
  elapsedS: number;
}

export type RepeatMode = "off" | "all" | "one";

export interface SyncInfo {
  running: boolean;
  pairUrl: string | null;
  /** SVG completo do QR — vai num dangerouslySetInnerHTML. */
  qrSvg: string | null;
}

export type SyncEvent =
  | { kind: "listening" }
  | { kind: "peerConnected"; name: string }
  | { kind: "sending"; peer: string; done: number; total: number }
  | { kind: "peerFinished" }
  | { kind: "error"; msg: string };

export interface Playback {
  playing: boolean;
  positionMs: number;
  durationMs: number | null;
  now: TrackRow | null;
  queuePos: number | null;
  queueLen: number;
  shuffle: boolean;
  repeat: RepeatMode;
  volume: number;
}

export const api = {
  stats: () => invoke<Stats>("library_stats"),
  currentRoot: () => invoke<string | null>("current_root"),
  pickFolder: () => invoke<string | null>("pick_folder"),
  rescan: () => invoke<void>("rescan"),
  playlists: () => invoke<Playlist[]>("list_playlists"),
  artists: () => invoke<Artist[]>("list_artists"),
  openSource: (source: Source, sort: Sort, query: string) =>
    invoke<OpenResult>("open_source", { source, sort, query }),
  trackRows: (start: number, count: number) =>
    invoke<TrackRow[]>("track_rows", { start, count }),

  playAt: (index: number) => invoke<Playback>("play_at", { index }),
  playPause: () => invoke<Playback>("play_pause"),
  nextTrack: () => invoke<Playback>("next_track"),
  prevTrack: () => invoke<Playback>("prev_track"),
  seek: (ms: number) => invoke<Playback>("seek", { ms }),
  setVolume: (volume: number) => invoke<void>("set_volume", { volume }),
  setShuffle: (on: boolean) => invoke<Playback>("set_shuffle", { on }),
  cycleRepeat: () => invoke<Playback>("cycle_repeat"),
  playbackSnapshot: () => invoke<Playback>("playback_snapshot"),
  flushPendingPlay: () => invoke<Playback | null>("flush_pending_play"),

  playlistCreate: (name?: string) => invoke<Playlist>("playlist_create", { name: name ?? null }),
  playlistRename: (id: string, name: string) => invoke<void>("playlist_rename", { id, name }),
  playlistDelete: (id: string) => invoke<void>("playlist_delete", { id }),
  playlistAddTracks: (id: string, tracks: number[]) =>
    invoke<number>("playlist_add_tracks", { id, tracks }),
  playlistRemoveAt: (index: number) => invoke<void>("playlist_remove_at", { index }),
  playlistMove: (from: number, to: number) => invoke<void>("playlist_move", { from, to }),
  playlistSetImage: (id: string) => invoke<void>("playlist_set_image", { id }),
  playlistClearImage: (id: string) => invoke<void>("playlist_clear_image", { id }),
  librarySetImage: () => invoke<string | null>("library_set_image"),
  libraryClearImage: () => invoke<void>("library_clear_image"),
  libraryImage: () => invoke<string | null>("library_image"),
  trackSetAlbumArt: (id: number) => invoke<void>("track_set_album_art", { id }),
  playlistLinks: (id: string) => invoke<LinkInfo[]>("playlist_links", { id }),
  playlistLinkFolder: (id: string) => invoke<void>("playlist_link_folder", { id }),
  playlistUnlinkFolder: (id: string, rootId: number, relPrefix: string) =>
    invoke<void>("playlist_unlink_folder", { id, rootId, relPrefix }),
  artistSetImage: (id: number) => invoke<void>("artist_set_image", { id }),
  artistClearImage: (id: number) => invoke<void>("artist_clear_image", { id }),

  syncStart: () => invoke<SyncInfo>("sync_start"),
  syncStop: () => invoke<void>("sync_stop"),
  syncInfo: () => invoke<SyncInfo>("sync_info"),
};

export interface LinkInfo {
  rootId: number;
  relPrefix: string;
  label: string;
}

export const onScanProgress = (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
  listen<ScanProgress>("scan://progress", (e) => cb(e.payload));
export const onScanDone = (cb: (d: ScanDone) => void): Promise<UnlistenFn> =>
  listen<ScanDone>("scan://done", (e) => cb(e.payload));
export const onScanError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
  listen<string>("scan://error", (e) => cb(e.payload));
export const onPlaybackState = (cb: (p: Playback) => void): Promise<UnlistenFn> =>
  listen<Playback>("playback://state", (e) => cb(e.payload));
export const onSyncEvent = (cb: (e: SyncEvent) => void): Promise<UnlistenFn> =>
  listen<SyncEvent>("sync://event", (e) => cb(e.payload));

/** URL da miniatura pro `<img>`. O esquema custom muda de forma por plataforma. */
export function artUrl(hash: string, size: 96 | 512): string {
  const isWindows = navigator.userAgent.includes("Windows");
  return isWindows
    ? `http://art.localhost/${hash}/${size}`
    : `art://localhost/${hash}/${size}`;
}
