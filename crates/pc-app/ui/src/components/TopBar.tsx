import { useEffect, useRef, useState } from "react";
import { appWindow } from "../lib/window";
import { useStore } from "../store";
import { Folder, RefreshCw, Search, Minus, Square, X, Smartphone } from "./icons";

function shortPath(p: string): string {
  const home = "/home/";
  if (p.startsWith(home)) {
    const rest = p.slice(home.length);
    const slash = rest.indexOf("/");
    return slash >= 0 ? "~/" + rest.slice(slash + 1) : p;
  }
  return p;
}

export function TopBar() {
  const root = useStore((s) => s.root);
  const query = useStore((s) => s.query);
  const setQuery = useStore((s) => s.setQuery);
  const pickFolder = useStore((s) => s.pickFolder);
  const rescan = useStore((s) => s.rescan);
  const openSync = useStore((s) => s.openSync);

  // debounce da busca: o store dispara open_source a cada mudança, mas a
  // digitação não deve martelar o backend.
  const [text, setText] = useState(query);
  const t = useRef<number | undefined>(undefined);
  useEffect(() => setText(query), [query]);
  function onType(v: string) {
    setText(v);
    window.clearTimeout(t.current);
    t.current = window.setTimeout(() => setQuery(v), 180);
  }

  return (
    <div className="topbar" data-tauri-drag-region>
      <div className="topbar-left">
        <button
          className="icobtn"
          title="Choose music folder"
          type="button"
          onClick={() => void pickFolder()}
        >
          <Folder />
        </button>
        <span className="path" title={root ?? undefined}>
          {root ? shortPath(root) : "No folder"}
        </span>
        <button
          className="icobtn"
          title="Rescan folder"
          type="button"
          disabled={!root}
          onClick={() => void rescan()}
        >
          <RefreshCw />
        </button>
        <button
          className="icobtn"
          title="Sync to phone"
          type="button"
          disabled={!root}
          onClick={() => void openSync()}
        >
          <Smartphone />
        </button>
      </div>

      <label className="search">
        <Search />
        <input
          type="text"
          placeholder="Search your library"
          spellCheck={false}
          value={text}
          onChange={(e) => onType(e.target.value)}
        />
      </label>

      <div className="topbar-right">
        <div className="wbtns">
          <button
            className="wbtn"
            title="Minimize"
            type="button"
            onClick={() => void appWindow.minimize()}
          >
            <Minus />
          </button>
          <button
            className="wbtn"
            title="Maximize"
            type="button"
            onClick={() => void appWindow.toggleMaximize()}
          >
            <Square />
          </button>
          <button
            className="wbtn close"
            title="Close"
            type="button"
            onClick={() => void appWindow.close()}
          >
            <X />
          </button>
        </div>
      </div>
    </div>
  );
}
