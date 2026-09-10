import { useEffect, useRef } from "react";
import { api } from "../lib/api";
import { useStore } from "../store";
import { Plus, LibraryBig } from "./icons";
import { Thumb } from "./Thumb";
import { openContextMenu, type MenuItem } from "./ContextMenu";

function RenameRow({ id, name }: { id: string; name: string }) {
  const renamePlaylist = useStore((s) => s.renamePlaylist);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      className="rename-input"
      defaultValue={name}
      spellCheck={false}
      onBlur={(e) => void renamePlaylist(id, e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          e.currentTarget.value = name;
          e.currentTarget.blur();
        }
      }}
    />
  );
}

export function Sidebar() {
  const stats = useStore((s) => s.stats);
  const playlists = useStore((s) => s.playlists);
  const artists = useStore((s) => s.artists);
  const libraryImage = useStore((s) => s.libraryImage);
  const sideTab = useStore((s) => s.sideTab);
  const source = useStore((s) => s.source);
  const renaming = useStore((s) => s.renaming);
  const setSideTab = useStore((s) => s.setSideTab);
  const openSource = useStore((s) => s.openSource);
  const s = useStore.getState;

  const libActive = source.kind === "library";
  const libSub = stats
    ? `${stats.tracks.toLocaleString()} tracks · ${stats.albums.toLocaleString()} albums`
    : "…";

  const libraryMenu: MenuItem[] = [
    { label: libraryImage ? "Change photo…" : "Choose photo…", onSelect: () => void s().setLibraryImage() },
    ...(libraryImage ? [{ label: "Remove photo", onSelect: () => void s().clearLibraryImage() }] : []),
  ];

  return (
    <aside className="card side">
      <div className="side-head">
        <h2>Your Library</h2>
        <button
          className="icobtn"
          title="New playlist"
          type="button"
          onClick={() => void s().createPlaylist()}
        >
          <Plus />
        </button>
      </div>

      <div className="rows">
        <button
          className={`row${libActive ? " on" : ""}`}
          type="button"
          onClick={() => void openSource({ kind: "library" })}
          onContextMenu={(e) => openContextMenu(e, libraryMenu)}
        >
          <Thumb
            covers={libraryImage ? [libraryImage] : []}
            seed="library"
            glyph={libraryImage ? undefined : <LibraryBig />}
          />
          <div className="r-txt">
            <div className="r-name">Your Library</div>
            <div className="r-sub">{libSub}</div>
          </div>
        </button>

        <div className="side-sep" />

        <div className="chips">
          <button
            className={`chip${sideTab === "playlists" ? " on" : ""}`}
            type="button"
            onClick={() => setSideTab("playlists")}
          >
            Playlists
          </button>
          <button
            className={`chip${sideTab === "artists" ? " on" : ""}`}
            type="button"
            onClick={() => setSideTab("artists")}
          >
            Artists
          </button>
        </div>

        {sideTab === "playlists" &&
          playlists.map((pl) => {
            if (renaming === pl.id) return <RenameRow key={pl.id} id={pl.id} name={pl.name} />;
            const on = source.kind === "playlist" && source.id === pl.id;
            const menu: MenuItem[] = [
              { label: "Rename", onSelect: () => s().startRename(pl.id) },
              {
                label: pl.hasImage ? "Change photo…" : "Choose photo…",
                onSelect: () => void s().setPlaylistImage(pl.id),
              },
              ...(pl.hasImage
                ? [{ label: "Remove photo", onSelect: () => void s().clearPlaylistImage(pl.id) }]
                : []),
              { label: "Delete", danger: true, onSelect: () => void s().deletePlaylist(pl.id) },
              { kind: "sep" as const },
              { label: "Link folder…", onSelect: () => void s().linkPlaylistFolder(pl.id) },
              ...(pl.linked
                ? [
                    {
                      label: "Unlink folder",
                      onSelect: async () => {
                        const links = await api.playlistLinks(pl.id);
                        const l = links[0];
                        if (l) await s().unlinkPlaylistFolder(pl.id, l.rootId, l.relPrefix);
                      },
                    },
                  ]
                : []),
            ];
            return (
              <button
                className={`row${on ? " on" : ""}`}
                key={pl.id}
                type="button"
                onClick={() => void openSource({ kind: "playlist", id: pl.id })}
                onContextMenu={(e) => openContextMenu(e, menu)}
              >
                <Thumb covers={pl.covers} seed={pl.id} />
                <div className="r-txt">
                  <div className="r-name">{pl.name}</div>
                  <div className="r-sub">
                    Playlist · {pl.items} tracks{pl.linked ? " · linked folder" : ""}
                  </div>
                </div>
              </button>
            );
          })}
        {sideTab === "playlists" && playlists.length === 0 && (
          <div className="side-label">No playlists yet</div>
        )}

        {sideTab === "artists" &&
          artists.map((ar) => {
            const on = source.kind === "artist" && source.id === ar.id;
            const menu: MenuItem[] = [
              {
                label: ar.hasImage ? "Change photo…" : "Choose photo…",
                onSelect: () => void s().setArtistImage(ar.id),
              },
              ...(ar.hasImage
                ? [{ label: "Remove photo", onSelect: () => void s().clearArtistImage(ar.id) }]
                : []),
            ];
            return (
              <button
                className={`row${on ? " on" : ""}`}
                key={ar.id}
                type="button"
                onClick={() => void openSource({ kind: "artist", id: ar.id })}
                onContextMenu={(e) => openContextMenu(e, menu)}
              >
                <Thumb covers={ar.cover ? [ar.cover] : []} seed={`artist-${ar.id}`} round />
                <div className="r-txt">
                  <div className="r-name">{ar.name}</div>
                  <div className="r-sub">
                    Artist · {ar.tracks} {ar.tracks === 1 ? "track" : "tracks"}
                  </div>
                </div>
              </button>
            );
          })}
        {sideTab === "artists" && artists.length === 0 && (
          <div className="side-label">No artists indexed</div>
        )}
      </div>
    </aside>
  );
}
