import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "../lib/window";

/* Janela sem decoração não tem borda de redimensionar. Estas oito faixas
   finas no perímetro chamam `startResizeDragging` — o WM cuida do resto. */

type Dir =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

const HANDLES: { dir: Dir; cls: string }[] = [
  { dir: "North", cls: "rh-n" },
  { dir: "South", cls: "rh-s" },
  { dir: "East", cls: "rh-e" },
  { dir: "West", cls: "rh-w" },
  { dir: "NorthWest", cls: "rh-nw" },
  { dir: "NorthEast", cls: "rh-ne" },
  { dir: "SouthWest", cls: "rh-sw" },
  { dir: "SouthEast", cls: "rh-se" },
];

export function ResizeHandles() {
  if (!inTauri) return null;
  return (
    <div className="resize-handles" aria-hidden="true">
      {HANDLES.map(({ dir, cls }) => (
        <div
          key={dir}
          className={`rh ${cls}`}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            void getCurrentWindow().startResizeDragging(dir);
          }}
        />
      ))}
    </div>
  );
}
