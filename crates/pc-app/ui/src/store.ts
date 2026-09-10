import { create } from "zustand";
import { enterMiniWindow, exitMiniWindow } from "./lib/window";
import {
  api,
  onPlaybackState,
  onScanDone,
  onScanError,
  onScanProgress,
  type Artist,
  type OpenResult,
  type Playback,
  type Playlist,
  type Sort,
  type Source,
  type Stats,
} from "./lib/api";

type SideTab = "playlists" | "artists";

interface AppStore {
  ready: boolean;
  root: string | null;
  stats: Stats | null;
  playlists: Playlist[];
  artists: Artist[];
  libraryImage: string | null;
  sideTab: SideTab;

  source: Source;
  sort: Sort;
  query: string;
  open: OpenResult | null;
  total: number;

  /** Playlist em edição de nome na sidebar: id. */
  renaming: string | null;

  scan: { active: boolean; done: number } | null;
  toast: string | null;
  playback: Playback | null;

  mini: boolean;
  normalSize: [number, number] | null;
  toggleMini: () => Promise<void>;

  init: () => Promise<void>;
  refreshLibrary: () => Promise<void>;
  reopen: () => Promise<void>;
  openSource: (s: Source) => Promise<void>;
  setSideTab: (t: SideTab) => void;
  setQuery: (q: string) => void;
  pickFolder: () => Promise<void>;
  rescan: () => Promise<void>;

  playAt: (index: number) => Promise<void>;
  playPause: () => Promise<void>;
  next: () => Promise<void>;
  prev: () => Promise<void>;
  seek: (ms: number) => Promise<void>;
  seekBy: (deltaMs: number) => Promise<void>;
  setVolume: (v: number) => Promise<void>;
  toggleShuffle: () => Promise<void>;
  cycleRepeat: () => Promise<void>;

  createPlaylist: (track?: number) => Promise<void>;
  renamePlaylist: (id: string, name: string) => Promise<void>;
  startRename: (id: string) => void;
  deletePlaylist: (id: string) => Promise<void>;
  addToPlaylist: (id: string, track: number) => Promise<void>;
  removeFromPlaylist: (index: number) => Promise<void>;
  movePlaylistItem: (from: number, to: number) => Promise<void>;
  setPlaylistImage: (id: string) => Promise<void>;
  clearPlaylistImage: (id: string) => Promise<void>;
  setLibraryImage: () => Promise<void>;
  clearLibraryImage: () => Promise<void>;
  linkPlaylistFolder: (id: string) => Promise<void>;
  unlinkPlaylistFolder: (id: string, rootId: number, relPrefix: string) => Promise<void>;
  setAlbumArt: (trackId: number) => Promise<void>;
  viewArtist: (id: number) => Promise<void>;
}

function sameSource(a: Source, b: Source): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "playlist" && b.kind === "playlist") return a.id === b.id;
  if (a.kind === "artist" && b.kind === "artist") return a.id === b.id;
  return true;
}

const flash = (msg: string, set: (p: Partial<AppStore>) => void) => {
  set({ toast: msg });
  window.setTimeout(() => set({ toast: null }), 6000);
};

export const useStore = create<AppStore>((set, get) => ({
  ready: false,
  root: null,
  stats: null,
  playlists: [],
  artists: [],
  libraryImage: null,
  sideTab: "playlists",
  source: { kind: "library" },
  sort: "artist-album",
  query: "",
  open: null,
  total: 0,
  renaming: null,
  scan: null,
  toast: null,
  playback: null,
  mini: false,
  normalSize: null,

  toggleMini: async () => {
    try {
      if (get().mini) {
        await exitMiniWindow(get().normalSize);
        set({ mini: false });
      } else {
        const normal = await enterMiniWindow();
        set({ mini: true, normalSize: normal });
      }
    } catch (e) {
      flash(`mini: ${String(e)}`, set);
    }
  },

  init: async () => {
    onPlaybackState((p) => set({ playback: p }));
    onScanProgress((p) => set({ scan: { active: true, done: p.done } }));
    onScanDone(async (d) => {
      set({
        scan: null,
        toast:
          d.added + d.updated + d.removed > 0
            ? `${d.added} new · ${d.updated} updated · ${d.removed} removed`
            : null,
      });
      await get().refreshLibrary();
      const played = await api.flushPendingPlay();
      if (played) set({ playback: played });
      await get().reopen();
      window.setTimeout(() => set({ toast: null }), 6000);
    });
    onScanError((msg) => set({ scan: null, toast: `scan failed: ${msg}` }));

    const [root, playback] = await Promise.all([api.currentRoot(), api.playbackSnapshot()]);
    set({ root, playback });
    if (root) {
      await get().refreshLibrary();
      await get().openSource({ kind: "library" });
    }
    set({ ready: true });
  },

  refreshLibrary: async () => {
    const [stats, playlists, artists, libraryImage] = await Promise.all([
      api.stats(),
      api.playlists(),
      api.artists(),
      api.libraryImage(),
    ]);
    set({ stats, playlists, artists, libraryImage });
  },

  reopen: async () => {
    const s = get().source;
    // se a fonte sumiu (playlist apagada), volta pra biblioteca
    if (s.kind === "playlist" && !get().playlists.some((p) => p.id === s.id)) {
      await get().openSource({ kind: "library" });
    } else {
      await get().openSource(s);
    }
  },

  openSource: async (s) => {
    const { sort, query } = get();
    const q = s.kind === "library" ? query : "";
    const open = await api.openSource(s, sort, q);
    set({ source: s, open, total: open.total });
  },

  setSideTab: (t) => set({ sideTab: t }),

  setQuery: (q) => {
    set({ query: q });
    const s = get().source;
    if (s.kind === "library") void get().openSource(s);
  },

  pickFolder: async () => {
    const picked = await api.pickFolder();
    if (picked) set({ root: picked, scan: { active: true, done: 0 } });
  },

  rescan: async () => {
    if (!get().root) return;
    set({ scan: { active: true, done: 0 } });
    await api.rescan();
  },

  playAt: async (index) => set({ playback: await api.playAt(index) }),
  playPause: async () => set({ playback: await api.playPause() }),
  next: async () => set({ playback: await api.nextTrack() }),
  prev: async () => set({ playback: await api.prevTrack() }),
  seek: async (ms) => set({ playback: await api.seek(Math.max(0, Math.round(ms))) }),
  seekBy: async (deltaMs) => {
    const p = get().playback;
    if (!p || p.durationMs == null) return;
    await get().seek(Math.min(p.durationMs, p.positionMs + deltaMs));
  },
  setVolume: async (v) => {
    const vol = Math.min(1, Math.max(0, v));
    set((s) => (s.playback ? { playback: { ...s.playback, volume: vol } } : {}));
    await api.setVolume(vol);
  },
  toggleShuffle: async () => {
    const p = get().playback;
    set({ playback: await api.setShuffle(!(p?.shuffle ?? false)) });
  },
  cycleRepeat: async () => set({ playback: await api.cycleRepeat() }),

  createPlaylist: async (track) => {
    const pl = await api.playlistCreate();
    if (track != null) await api.playlistAddTracks(pl.id, [track]);
    await get().refreshLibrary();
    set({ sideTab: "playlists", renaming: pl.id });
  },
  startRename: (id) => set({ renaming: id }),
  renamePlaylist: async (id, name) => {
    set({ renaming: null });
    const trimmed = name.trim();
    if (trimmed) await api.playlistRename(id, trimmed);
    await get().refreshLibrary();
    if (get().source.kind === "playlist" && (get().source as { id: string }).id === id) {
      await get().reopen();
    }
  },
  deletePlaylist: async (id) => {
    await api.playlistDelete(id);
    await get().refreshLibrary();
    await get().reopen();
  },
  addToPlaylist: async (id, track) => {
    const n = await api.playlistAddTracks(id, [track]);
    await get().refreshLibrary();
    flash(n > 0 ? "Added to playlist" : "Already in playlist", set);
  },
  removeFromPlaylist: async (index) => {
    await api.playlistRemoveAt(index);
    await get().reopen();
    await get().refreshLibrary();
  },
  movePlaylistItem: async (from, to) => {
    if (from === to) return;
    await api.playlistMove(from, to);
    await get().reopen();
  },
  setPlaylistImage: async (id) => {
    await api.playlistSetImage(id);
    await get().refreshLibrary();
    await get().reopen();
  },
  clearPlaylistImage: async (id) => {
    await api.playlistClearImage(id);
    await get().refreshLibrary();
    await get().reopen();
  },
  setLibraryImage: async () => {
    const hash = await api.librarySetImage();
    if (hash) set({ libraryImage: hash });
  },
  clearLibraryImage: async () => {
    await api.libraryClearImage();
    set({ libraryImage: null });
  },
  linkPlaylistFolder: async (id) => {
    try {
      await api.playlistLinkFolder(id);
      await get().refreshLibrary();
      await get().reopen();
      flash("Folder linked", set);
    } catch (e) {
      flash(String(e), set);
    }
  },
  unlinkPlaylistFolder: async (id, rootId, relPrefix) => {
    await api.playlistUnlinkFolder(id, rootId, relPrefix);
    await get().refreshLibrary();
    flash("Folder unlinked", set);
  },
  setAlbumArt: async (trackId) => {
    await api.trackSetAlbumArt(trackId);
    await get().refreshLibrary();
    await get().reopen();
  },
  viewArtist: async (id) => {
    await get().openSource({ kind: "artist", id });
  },
}));

export { sameSource };
