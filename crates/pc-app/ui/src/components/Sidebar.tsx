import { useStore } from "../store";
import { Plus, LibraryBig } from "./icons";
import { Thumb } from "./Thumb";

export function Sidebar() {
  const stats = useStore((s) => s.stats);
  const playlists = useStore((s) => s.playlists);
  const artists = useStore((s) => s.artists);
  const sideTab = useStore((s) => s.sideTab);
  const source = useStore((s) => s.source);
  const setSideTab = useStore((s) => s.setSideTab);
  const openSource = useStore((s) => s.openSource);

  const libActive = source.kind === "library";
  const libSub = stats
    ? `${stats.tracks.toLocaleString()} tracks · ${stats.albums.toLocaleString()} albums`
    : "…";

  return (
    <aside className="card side">
      <div className="side-head">
        <h2>Your Library</h2>
        <button className="icobtn" title="New playlist" type="button">
          <Plus />
        </button>
      </div>

      <div className="rows">
        <button
          className={`row${libActive ? " on" : ""}`}
          type="button"
          onClick={() => void openSource({ kind: "library" })}
        >
          <Thumb covers={[]} seed="library" glyph={<LibraryBig />} />
          <div className="r-txt">
            <div className="r-name">Your Library</div>
            <div className="r-sub">{libSub}</div>
          </div>
        </button>

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
            const on = source.kind === "playlist" && source.id === pl.id;
            return (
              <button
                className={`row${on ? " on" : ""}`}
                key={pl.id}
                type="button"
                onClick={() => void openSource({ kind: "playlist", id: pl.id })}
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
            return (
              <button
                className={`row${on ? " on" : ""}`}
                key={ar.id}
                type="button"
                onClick={() => void openSource({ kind: "artist", id: ar.id })}
              >
                <Thumb covers={[]} seed={`artist-${ar.id}`} round />
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
