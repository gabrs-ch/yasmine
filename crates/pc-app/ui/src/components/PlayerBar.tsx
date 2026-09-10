import {
  Shuffle,
  Repeat,
  PictureInPicture2,
  Volume2,
  PrevIcon,
  NextIcon,
  PauseIcon,
} from "./icons";

/* Fase 1: barra estática do mockup. Fase 3 liga no Engine/Queue (estado por
   evento `playback://state`, cliques por comando). */
export function PlayerBar() {
  return (
    <div className="player">
      <div className="np">
        <div className="np-cover a5" />
        <div className="np-txt">
          <div className="np-title">Have You Seen Me Dance Alone?</div>
          <div className="np-sub">TOMORA, AURORA · HAVE YOU SEEN ME DANCE ALONE?</div>
        </div>
      </div>

      <div className="center">
        <div className="transport">
          <button className="tbtn toggle-underline on" title="Shuffle" type="button">
            <Shuffle />
          </button>
          <button className="tbtn" title="Previous" type="button">
            <PrevIcon />
          </button>
          <button className="playbtn" title="Pause" type="button">
            <PauseIcon />
          </button>
          <button className="tbtn" title="Next" type="button">
            <NextIcon />
          </button>
          <button className="tbtn toggle-underline on" title="Repeat: all" type="button">
            <Repeat />
          </button>
        </div>
        <div className="bar">
          <span className="time">2:04</span>
          <span className="track-line">
            <span className="track-fill" style={{ width: "34%" }} />
            <span className="track-knob" style={{ left: "34%" }} />
          </span>
          <span className="time">4:21</span>
        </div>
      </div>

      <div className="right">
        <span className="qcount">4 / 42</span>
        <button className="tbtn toggle-underline" title="Compact mode" type="button">
          <PictureInPicture2 />
        </button>
        <span className="vol">
          <Volume2 />
          <span className="vol-line">
            <span className="vol-fill" style={{ width: "70%" }} />
          </span>
        </span>
      </div>
    </div>
  );
}
