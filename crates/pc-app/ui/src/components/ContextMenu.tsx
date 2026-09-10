import { useEffect, useRef, useState } from "react";

/* Menu de contexto flutuante, estilizado como o resto do app (o menu nativo
   do WebKit destoaria). Um host no root; `openContextMenu(e, items)` de
   qualquer lugar. Submenu abre no hover. */

export type MenuItem =
  | { kind: "sep" }
  | { kind: "label"; text: string }
  | {
      label: string;
      onSelect?: () => void;
      submenu?: MenuItem[];
      danger?: boolean;
      disabled?: boolean;
    };

type Anchor = { x: number; y: number; items: MenuItem[] };

let openFn: ((a: Anchor) => void) | null = null;

export function openContextMenu(
  e: { preventDefault(): void; stopPropagation(): void; clientX: number; clientY: number },
  items: MenuItem[],
) {
  e.preventDefault();
  e.stopPropagation();
  openFn?.({ x: e.clientX, y: e.clientY, items });
}

function Menu({ items, x, y, onClose }: Anchor & { onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  const [openSub, setOpenSub] = useState<number | null>(null);

  useEffect(() => {
    // reposiciona pra caber na viewport
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const nx = x + r.width > window.innerWidth ? Math.max(4, x - r.width) : x;
    const ny = y + r.height > window.innerHeight ? Math.max(4, y - r.height) : y;
    setPos({ x: nx, y: ny });
  }, [x, y]);

  return (
    <div
      ref={ref}
      className="ctxmenu"
      style={{ left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((it, i) => {
        if ("kind" in it && it.kind === "sep") return <div key={i} className="ctx-sep" />;
        if ("kind" in it && it.kind === "label")
          return (
            <div key={i} className="ctx-label">
              {it.text}
            </div>
          );
        const item = it as Extract<MenuItem, { label: string }>;
        const hasSub = !!item.submenu?.length;
        return (
          <div
            key={i}
            className={`ctx-item${item.danger ? " danger" : ""}${item.disabled ? " disabled" : ""}`}
            onMouseEnter={() => setOpenSub(hasSub ? i : null)}
            onClick={() => {
              if (item.disabled || hasSub) return;
              item.onSelect?.();
              onClose();
            }}
          >
            <span>{item.label}</span>
            {hasSub && <span className="ctx-caret">›</span>}
            {hasSub && openSub === i && (
              <div className="ctx-sub">
                <Menu items={item.submenu!} x={0} y={0} onClose={onClose} />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function ContextMenuHost() {
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  useEffect(() => {
    openFn = setAnchor;
    return () => {
      openFn = null;
    };
  }, []);
  useEffect(() => {
    if (!anchor) return;
    const close = () => setAnchor(null);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    // clique fora / scroll / blur fecham
    window.addEventListener("pointerdown", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey);
    document.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("scroll", close, true);
    };
  }, [anchor]);

  if (!anchor) return null;
  return (
    <div
      className="ctx-overlay"
      onPointerDown={(e) => {
        // deixa o pointerdown global fechar; mas impede que ele borbulhe pro app
        e.stopPropagation();
        if (e.target === e.currentTarget) setAnchor(null);
      }}
    >
      <Menu {...anchor} onClose={() => setAnchor(null)} />
    </div>
  );
}
