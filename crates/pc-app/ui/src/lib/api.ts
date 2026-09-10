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
}

export interface Artist {
  id: number;
  name: string;
  tracks: number;
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
};

export const onScanProgress = (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
  listen<ScanProgress>("scan://progress", (e) => cb(e.payload));
export const onScanDone = (cb: (d: ScanDone) => void): Promise<UnlistenFn> =>
  listen<ScanDone>("scan://done", (e) => cb(e.payload));
export const onScanError = (cb: (msg: string) => void): Promise<UnlistenFn> =>
  listen<string>("scan://error", (e) => cb(e.payload));

/** URL da miniatura pro `<img>`. O esquema custom muda de forma por plataforma. */
export function artUrl(hash: string, size: 96 | 512): string {
  const isWindows = navigator.userAgent.includes("Windows");
  return isWindows
    ? `http://art.localhost/${hash}/${size}`
    : `art://localhost/${hash}/${size}`;
}
