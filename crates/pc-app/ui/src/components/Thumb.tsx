import type { ReactNode } from "react";
import { artUrl } from "../lib/api";

/** `a1`…`a8` determinístico a partir de uma seed (id da playlist/artista ou
 *  hash da capa) — mesmo esquema do mockup e do egui (`tile_gradient`). */
export function coverClass(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) | 0;
  return `a${(Math.abs(h) % 8) + 1}`;
}

function Tile({ hash, seed }: { hash?: string; seed: string }) {
  return (
    <div className={`tile ${coverClass(seed)}`}>
      {hash && (
        <img
          src={artUrl(hash, 96)}
          alt=""
          loading="lazy"
          onError={(e) => {
            e.currentTarget.style.display = "none";
          }}
        />
      )}
    </div>
  );
}

interface ThumbProps {
  covers: string[];
  seed: string;
  round?: boolean;
  glyph?: ReactNode;
}

export function Thumb({ covers, seed, round = false, glyph }: ThumbProps) {
  const base = `thumb${round ? " round" : ""}`;
  if (glyph) return <div className={`${base} lib`}>{glyph}</div>;
  if (covers.length >= 4) {
    return (
      <div className={`${base} grid`}>
        {covers.slice(0, 4).map((h, i) => (
          <Tile key={i} hash={h} seed={seed + i} />
        ))}
      </div>
    );
  }
  return (
    <div className={base} style={{ overflow: "hidden" }}>
      <Tile hash={covers[0]} seed={seed} />
    </div>
  );
}
