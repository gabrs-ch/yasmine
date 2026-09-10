import { useRef, useState } from "react";
import { artUrl } from "../lib/api";
import { useStore } from "../store";
import { coverClass } from "./Thumb";
import {
  Shuffle,
  Repeat,
  Repeat1,
  PictureInPicture2,
  Volume2,
  PrevIcon,
  NextIcon,
  PlayIcon,
  PauseIcon,
} from "./icons";

function fmt(ms: number | null): string {
  if (ms == null) return "--:--";
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

interface ScrubberProps {
  value: number; // 0..1
  className: string;
  fillClass: string;
  knob?: boolean;
  live?: boolean; // dispara onSeek durante o arraste (usado no volume)
  onSeek: (frac: number) => void;
}

function Scrubber({ value, className, fillClass, knob, live, onSeek }: ScrubberProps) {
  const ref = useRef<HTMLSpanElement>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const shown = drag ?? value;

  const fracFrom = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width));
  };

  return (
    <span
      ref={ref}
      className={className}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        const f = fracFrom(e.clientX);
        setDrag(f);
        if (live) onSeek(f);
      }}
      onPointerMove={(e) => {
        if (drag === null) return;
        const f = fracFrom(e.clientX);
        setDrag(f);
        if (live) onSeek(f);
      }}
      onPointerUp={(e) => {
        if (drag === null) return;
        const f = fracFrom(e.clientX);
        setDrag(null);
        onSeek(f);
      }}
      onPointerCancel={() => setDrag(null)}
    >
      <span className={fillClass} style={{ width: `${shown * 100}%` }} />
      {knob && <span className="track-knob" style={{ left: `${shown * 100}%` }} />}
    </span>
  );
}

export function PlayerBar() {
  const pb = useStore((s) => s.playback);
  const playPause = useStore((s) => s.playPause);
  const next = useStore((s) => s.next);
  const prev = useStore((s) => s.prev);
  const seek = useStore((s) => s.seek);
  const setVolume = useStore((s) => s.setVolume);
  const toggleShuffle = useStore((s) => s.toggleShuffle);
  const cycleRepeat = useStore((s) => s.cycleRepeat);
  const toggleMini = useStore((s) => s.toggleMini);

  const now = pb?.now ?? null;
  const dur = pb?.durationMs ?? null;
  const pos = pb?.positionMs ?? 0;
  const frac = dur ? Math.min(1, pos / dur) : 0;
  const repeat = pb?.repeat ?? "off";

  return (
    <div className="player">
      <div className="np">
        {now ? (
          <div className={`np-cover ${coverClass(now.art ?? String(now.id))}`}>
            {now.art && (
              <img
                src={artUrl(now.art, 512)}
                alt=""
                style={{ width: "100%", height: "100%", objectFit: "cover", borderRadius: 8, display: "block" }}
                onError={(e) => {
                  e.currentTarget.style.display = "none";
                }}
              />
            )}
          </div>
        ) : (
          <div className="np-cover" />
        )}
        <div className="np-txt">
          {now ? (
            <>
              <div className="np-title">{now.title}</div>
              <div className="np-sub">
                {(now.artist ?? "—") + (now.album ? ` · ${now.album}` : "")}
              </div>
            </>
          ) : (
            <div className="np-sub">Nothing playing</div>
          )}
        </div>
      </div>

      <div className="center">
        <div className="transport">
          <button
            className={`tbtn toggle-underline${pb?.shuffle ? " on" : ""}`}
            title="Shuffle"
            type="button"
            onClick={() => void toggleShuffle()}
          >
            <Shuffle />
          </button>
          <button className="tbtn" title="Previous" type="button" onClick={() => void prev()}>
            <PrevIcon />
          </button>
          <button
            className="playbtn"
            title={pb?.playing ? "Pause" : "Play"}
            type="button"
            onClick={() => void playPause()}
          >
            {pb?.playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <button className="tbtn" title="Next" type="button" onClick={() => void next()}>
            <NextIcon />
          </button>
          <button
            className={`tbtn toggle-underline${repeat !== "off" ? " on" : ""}`}
            title={`Repeat: ${repeat}`}
            type="button"
            onClick={() => void cycleRepeat()}
          >
            {repeat === "one" ? <Repeat1 /> : <Repeat />}
          </button>
        </div>
        <div className="bar">
          <span className="time">{fmt(now ? pos : null)}</span>
          <Scrubber
            className="track-line"
            fillClass="track-fill"
            knob
            value={frac}
            onSeek={(f) => {
              if (dur) void seek(f * dur);
            }}
          />
          <span className="time">{fmt(dur)}</span>
        </div>
      </div>

      <div className="right">
        {pb && pb.queueLen > 0 && (
          <span className="qcount">
            {pb.queuePos ?? "–"} / {pb.queueLen}
          </span>
        )}
        <button
          className="tbtn toggle-underline"
          title="Compact mode (Ctrl+M)"
          type="button"
          onClick={() => void toggleMini()}
        >
          <PictureInPicture2 />
        </button>
        <span className="vol">
          <Volume2 />
          <Scrubber
            className="vol-line"
            fillClass="vol-fill"
            live
            value={pb?.volume ?? 1}
            onSeek={(f) => void setVolume(f)}
          />
        </span>
      </div>
    </div>
  );
}
