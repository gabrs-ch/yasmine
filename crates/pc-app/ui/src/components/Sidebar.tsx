import { Plus, LibraryBig } from "./icons";

/* Fase 1: casca estática com os dados de exemplo do mockup. O conteúdo real
   (playlists, artistas, contagens) entra na Fase 2 via comandos Tauri. */

const PLACEHOLDER_PLAYLISTS = [
  { name: "Late Night Drive", sub: "Playlist · 42 tracks", cover: "a3" as const },
  { name: "Focus Mix", sub: "Playlist · 68 tracks · linked folder", grid: ["a1", "a6", "a4", "a2"] },
  { name: "wildlands", sub: "Playlist · 25 tracks", cover: "a5" as const },
  { name: "I've been for a walking", sub: "Playlist · 17 tracks", cover: "a8" as const },
  { name: "Rainy Sunday", sub: "Playlist · 31 tracks", cover: "a7" as const },
];

export function Sidebar() {
  return (
    <aside className="card side">
      <div className="side-head">
        <h2>Your Library</h2>
        <button className="icobtn" title="New playlist" type="button">
          <Plus />
        </button>
      </div>

      <div className="rows">
        <div className="row on">
          <div className="thumb lib">
            <LibraryBig />
          </div>
          <div className="r-txt">
            <div className="r-name">Your Library</div>
            <div className="r-sub">3,184 tracks · 291 albums</div>
          </div>
        </div>

        <div className="chips" style={{ padding: "10px 8px 6px" }}>
          <button className="chip on" type="button">
            Playlists
          </button>
          <button className="chip" type="button">
            Artists
          </button>
        </div>

        {PLACEHOLDER_PLAYLISTS.map((pl) => (
          <div className="row" key={pl.name}>
            {"grid" in pl && pl.grid ? (
              <div className="thumb grid">
                {pl.grid.map((g, i) => (
                  <div key={i} className={g} />
                ))}
              </div>
            ) : (
              <div className={`thumb ${pl.cover}`} />
            )}
            <div className="r-txt">
              <div className="r-name">{pl.name}</div>
              <div className="r-sub">{pl.sub}</div>
            </div>
          </div>
        ))}
      </div>
    </aside>
  );
}
