/* Ícones. A maioria vem do lucide-react (mesmos do mockup). Os quatro do
   transporte (prev/next/play/pause) são cheios e simples — ficam inline,
   iguais ao SVG do mockup, pra não depender do preenchimento do Lucide. */

export {
  Folder,
  RefreshCw,
  Search,
  Minus,
  Square,
  X,
  Plus,
  LibraryBig,
  Shuffle,
  Repeat,
  Repeat1,
  PictureInPicture2,
  Volume2,
  VolumeX,
  QrCode,
  Smartphone,
} from "lucide-react";

export function PrevIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M7 5v14H5V5h2Zm12 0v14l-11-7 11-7Z" />
    </svg>
  );
}

export function NextIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M17 5v14h2V5h-2ZM5 5v14l11-7L5 5Z" />
    </svg>
  );
}

export function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M8 5v14l11-7L8 5Z" />
    </svg>
  );
}

export function PauseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <rect x="6" y="5" width="4" height="14" rx="1" />
      <rect x="14" y="5" width="4" height="14" rx="1" />
    </svg>
  );
}

/** As três barrinhas de "isto está tocando". Animação e cor no CSS (.eq);
 *  `paused` congela as barras (playback em pausa). */
export function Equalizer({ paused = false }: { paused?: boolean }) {
  return (
    <span className={`eq${paused ? " paused" : ""}`} aria-hidden="true">
      <i />
      <i />
      <i />
    </span>
  );
}
