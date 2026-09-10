import { artUrl } from "../lib/api";
import { useStore } from "../store";
import { coverClass } from "./Thumb";
import { PrevIcon, NextIcon, PlayIcon, PauseIcon, PictureInPicture2 } from "./icons";

/* Modo compacto: a janela inteira é uma barrinha — capa, faixa, transporte.
   `data-tauri-drag-region` na área livre move a janela (sem decoração). */
export function MiniBar() {
  const pb = useStore((s) => s.playback);
  const playPause = useStore((s) => s.playPause);
  const next = useStore((s) => s.next);
  const prev = useStore((s) => s.prev);
  const toggleMini = useStore((s) => s.toggleMini);
  const now = pb?.now ?? null;

  return (
    <div className="mini" data-tauri-drag-region>
      <div className={`mini-cover ${coverClass(now?.art ?? "x")}`}>
        {now?.art && (
          <img
            src={artUrl(now.art, 96)}
            alt=""
            onError={(e) => (e.currentTarget.style.display = "none")}
          />
        )}
      </div>
      <div className="mini-txt" data-tauri-drag-region>
        <div className="mini-title">{now?.title ?? "Nothing playing"}</div>
        <div className="mini-sub">{now?.artist ?? ""}</div>
      </div>
      <div className="mini-transport">
        <button className="tbtn" type="button" title="Previous" onClick={() => void prev()}>
          <PrevIcon />
        </button>
        <button
          className="playbtn"
          type="button"
          title={pb?.playing ? "Pause" : "Play"}
          onClick={() => void playPause()}
        >
          {pb?.playing ? <PauseIcon /> : <PlayIcon />}
        </button>
        <button className="tbtn" type="button" title="Next" onClick={() => void next()}>
          <NextIcon />
        </button>
        <button
          className="tbtn"
          type="button"
          title="Exit compact mode"
          onClick={() => void toggleMini()}
        >
          <PictureInPicture2 />
        </button>
      </div>
    </div>
  );
}
