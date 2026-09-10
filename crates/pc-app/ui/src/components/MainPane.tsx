import { Equalizer } from "./icons";

/* Fase 1: hero + lista com os dados de exemplo do mockup. Na Fase 2 isto
   consome `list_tracks` / `track_rows` / `hero_art` com scroll virtualizado. */

type Row = {
  n: number;
  cover: string;
  title: string;
  artist: string;
  album: string;
  dur: string;
  playing?: boolean;
};

const PLACEHOLDER_ROWS: Row[] = [
  { n: 1, cover: "a4", title: "Blinding Lights", artist: "The Weeknd", album: "After Hours", dur: "3:20" },
  { n: 2, cover: "a8", title: "Runaway", artist: "AURORA", album: "All My Demons Greeting Me…", dur: "4:03" },
  { n: 3, cover: "a3", title: "Máquina do Tempo", artist: "Matuê", album: "Máquina do Tempo", dur: "3:47" },
  {
    n: 4,
    cover: "a5",
    title: "Have You Seen Me Dance Alone?",
    artist: "TOMORA, AURORA",
    album: "HAVE YOU SEEN ME DANCE ALONE?",
    dur: "4:21",
    playing: true,
  },
  { n: 5, cover: "a1", title: "Nightcall", artist: "Kavinsky", album: "OutRun", dur: "4:18" },
  { n: 6, cover: "a2", title: "The Stage", artist: "Avenged Sevenfold", album: "The Stage", dur: "8:32" },
  { n: 7, cover: "a6", title: "Redbone", artist: "Childish Gambino", album: "Awaken, My Love!", dur: "5:27" },
  {
    n: 8,
    cover: "a7",
    title: "Instant Crush",
    artist: "Daft Punk, Julian Casablancas",
    album: "Random Access Memories",
    dur: "5:37",
  },
  { n: 9, cover: "a4", title: "Out of Time", artist: "The Weeknd", album: "Dawn FM", dur: "3:34" },
];

export function MainPane() {
  return (
    <section className="card main">
      <div className="hero">
        <div className="hero-cover a3" />
        <div className="hero-meta">
          <div className="eyebrow">Playlist</div>
          <h1>Late Night Drive</h1>
          <div className="sub">
            <b>Gabriel</b> · 42 tracks · 2 hr 51 min
          </div>
        </div>
      </div>

      <div className="listwrap">
        <div className="lhead">
          <span>#</span>
          <span />
          <span>Title</span>
          <span>Artist</span>
          <span>Album</span>
          <span className="r">Time</span>
        </div>

        <div className="tracks">
          {PLACEHOLDER_ROWS.map((r) => (
            <div className={`track${r.playing ? " playing" : ""}`} key={r.n}>
              <div className="num" style={r.playing ? { display: "flex", alignItems: "center" } : undefined}>
                {r.playing ? <Equalizer /> : r.n}
              </div>
              <div className={`ca ${r.cover}`} />
              <div className="tt">{r.title}</div>
              <div className="ar">{r.artist}</div>
              <div className="al">{r.album}</div>
              <div className="du">{r.dur}</div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
