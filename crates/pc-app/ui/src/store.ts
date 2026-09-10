import { create } from "zustand";
import {
  api,
  onScanDone,
  onScanError,
  onScanProgress,
  type Artist,
  type OpenResult,
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
  sideTab: SideTab;

  source: Source;
  sort: Sort;
  query: string;
  open: OpenResult | null;
  /** contador de faixas na view atual (= open.total) */
  total: number;

  scan: { active: boolean; done: number } | null;
  toast: string | null;

  init: () => Promise<void>;
  refreshLibrary: () => Promise<void>;
  openSource: (s: Source) => Promise<void>;
  setSideTab: (t: SideTab) => void;
  setQuery: (q: string) => void;
  pickFolder: () => Promise<void>;
  rescan: () => Promise<void>;
}

function sameSource(a: Source, b: Source): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "playlist" && b.kind === "playlist") return a.id === b.id;
  if (a.kind === "artist" && b.kind === "artist") return a.id === b.id;
  return true;
}

export const useStore = create<AppStore>((set, get) => ({
  ready: false,
  root: null,
  stats: null,
  playlists: [],
  artists: [],
  sideTab: "playlists",
  source: { kind: "library" },
  sort: "artist-album",
  query: "",
  open: null,
  total: 0,
  scan: null,
  toast: null,

  init: async () => {
    // eventos do scan primeiro, pra não perder um que dispare no meio do load
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
      const s = get().source;
      await get().openSource(s);
      window.setTimeout(() => set({ toast: null }), 6000);
    });
    onScanError((msg) => set({ scan: null, toast: `scan failed: ${msg}` }));

    const root = await api.currentRoot();
    set({ root });
    if (root) {
      await get().refreshLibrary();
      await get().openSource({ kind: "library" });
    }
    set({ ready: true });
  },

  refreshLibrary: async () => {
    const [stats, playlists, artists] = await Promise.all([
      api.stats(),
      api.playlists(),
      api.artists(),
    ]);
    set({ stats, playlists, artists });
  },

  openSource: async (s) => {
    const { sort, query } = get();
    // busca só faz sentido na biblioteca
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
    if (picked) {
      set({ root: picked, scan: { active: true, done: 0 } });
    }
  },

  rescan: async () => {
    if (!get().root) return;
    set({ scan: { active: true, done: 0 } });
    await api.rescan();
  },
}));

export { sameSource };
