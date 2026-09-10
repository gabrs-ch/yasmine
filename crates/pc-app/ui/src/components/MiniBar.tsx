import { useRef, useState } from "react";
import { artUrl } from "../lib/api";
import { useStore } from "../store";
import { coverClass } from "./Thumb";
import { PrevIcon, NextIcon, PlayIcon, PauseIcon, PictureInPicture2 } from "./icons";

function fmt(ms: number | null): string {
  if (ms == null) return "--:--";
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/* Modo compacto: os WMs (xfwm4, KWin) não deixam a janela ficar abaixo de
   ~200px de altura, então ela é uma barra larga e baixa — capa à esquerda,
   faixa + progresso no meio, transporte à direita. Área livre arrasta. */
export function MiniBar() {
  const pb = useStore((s) => s.playback);
  const playPause = useStore((s) => s.playPause);
  const next = useStore((s) => s.next);
  const prev = useStore((s) => s.prev);
  const seek = useStore((s) => s.seek);
  const toggleMini = useStore((s) => s.toggleMini);

  const now = pb?.now ?? null;
  const dur = pb?.durationMs ?? null;
  const pos = pb?.positionMs ?? 0;
  const frac = dur ? Math.min(1, pos / dur) : 0;

  const lineRef = useRef<HTMLSpanElement>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const shown = drag ?? frac;
  const fracFrom = (clientX: number) => {
    const r = lineRef.current!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width));
  };

  return (
    <div className="mini" data-tauri-drag-region>
      <button
        className="mini-exit"
        type="button"
        title="Exit compact mode"
        onClick={() => void toggleMini()}
      >
        <PictureInPicture2 />
      </button>

      <div className={`mini-cover ${coverClass(now?.art ?? "x")}`}>
        {now?.art && (
          <img
            src={artUrl(now.art, 512)}
            alt=""
            onError={(e) => (e.currentTarget.style.display = "none")}
          />
        )}
      </div>

      <div className="mini-mid" data-tauri-drag-region>
        <div className="mini-title">{now?.title ?? "Nothing playing"}</div>
        <div className="mini-sub">
          {now ? (now.artist ?? "—") + (now.album ? ` · ${now.album}` : "") : ""}
        </div>
        <div className="mini-bar">
          <span className="time">{fmt(now ? pos : null)}</span>
          <span
            ref={lineRef}
            className="track-line"
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              setDrag(fracFrom(e.clientX));
            }}
            onPointerMove={(e) => drag !== null && setDrag(fracFrom(e.clientX))}
            onPointerUp={(e) => {
              if (drag === null) return;
              const f = fracFrom(e.clientX);
              setDrag(null);
              if (dur) void seek(f * dur);
            }}
            onPointerCancel={() => setDrag(null)}
          >
            <span className="track-fill" style={{ width: `${shown * 100}%` }} />
            <span className="track-knob" style={{ left: `${shown * 100}%` }} />
          </span>
          <span className="time">{fmt(dur)}</span>
        </div>
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
      </div>
    </div>
  );
}
