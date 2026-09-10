import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, artUrl, type TrackRow } from "../lib/api";
import { useStore } from "../store";
import { coverClass } from "./Thumb";

const ROW_H = 46;

function fmtDur(ms: number | null): string {
  if (ms == null) return "";
  const s = Math.round(ms / 1000);
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

export function MainPane() {
  const open = useStore((s) => s.open);
  const total = useStore((s) => s.total);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [rows, setRows] = useState<Map<number, TrackRow>>(new Map());

  // Trocar de fonte zera a janela carregada e volta o scroll pro topo.
  const openKey = open ? `${open.kind}:${open.title}:${total}` : "none";
  useEffect(() => {
    setRows(new Map());
    scrollRef.current?.scrollTo({ top: 0 });
  }, [openKey]);

  const virt = useVirtualizer({
    count: total,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 14,
  });
  const items = virt.getVirtualItems();

  // Carrega faixas em janelas conforme entram na viewport.
  const range = useMemo(() => {
    if (items.length === 0) return null;
    return { first: items[0].index, last: items[items.length - 1].index };
  }, [items]);

  useEffect(() => {
    if (!range) return;
    let missing = false;
    for (let i = range.first; i <= range.last; i++) {
      if (!rows.has(i)) {
        missing = true;
        break;
      }
    }
    if (!missing) return;
    const start = Math.max(0, range.first - 24);
    const count = range.last + 24 - start + 1;
    let cancelled = false;
    api.trackRows(start, count).then((got) => {
      if (cancelled) return;
      setRows((prev) => {
        const next = new Map(prev);
        got.forEach((r, i) => next.set(start + i, r));
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [range, rows]);

  const showHero = open != null && open.kind !== "library";

  return (
    <section className="card main">
      {showHero && open && (
        <div className="hero">
          {open.heroArt ? (
            <img className="hero-cover" src={artUrl(open.heroArt, 512)} alt="" />
          ) : (
            <div className={`hero-cover ${coverClass(open.title)}`} />
          )}
          <div className="hero-meta">
            <div className="eyebrow">{open.kind}</div>
            <h1>{open.title}</h1>
            <div className="sub">{open.subtitle}</div>
          </div>
        </div>
      )}

      <div className="listwrap">
        <div className="lhead">
          <span>#</span>
          <span />
          <span>Title</span>
          <span>Artist</span>
          <span>Album</span>
          <span className="r">Time</span>
        </div>

        <div className="tracks" ref={scrollRef}>
          <div style={{ height: virt.getTotalSize(), position: "relative", width: "100%" }}>
            {items.map((vi) => {
              const row = rows.get(vi.index);
              const style: React.CSSProperties = {
                position: "absolute",
                top: 0,
                left: 0,
                right: 0,
                height: ROW_H,
                transform: `translateY(${vi.start}px)`,
              };
              if (!row) {
                return (
                  <div className="track skeleton" style={style} key={vi.key}>
                    <div className="num">{vi.index + 1}</div>
                    <div className="ca" />
                    <div className="tt" />
                    <div className="ar" />
                    <div className="al" />
                    <div className="du" />
                  </div>
                );
              }
              return (
                <div className="track" style={style} key={vi.key}>
                  <div className="num">{row.trackNo ?? vi.index + 1}</div>
                  <div className={`ca ${coverClass(row.art ?? String(row.id))}`}>
                    {row.art && (
                      <img
                        src={artUrl(row.art, 96)}
                        alt=""
                        loading="lazy"
                        style={{ width: "100%", height: "100%", objectFit: "cover", borderRadius: 5, display: "block" }}
                        onError={(e) => {
                          e.currentTarget.style.display = "none";
                        }}
                      />
                    )}
                  </div>
                  <div className="tt">{row.title}</div>
                  <div className="ar">{row.artist ?? "—"}</div>
                  <div className="al">{row.album ?? "—"}</div>
                  <div className="du">{fmtDur(row.durationMs)}</div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </section>
  );
}
