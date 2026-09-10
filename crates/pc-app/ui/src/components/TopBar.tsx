import { getCurrentWindow } from "@tauri-apps/api/window";
import { Folder, RefreshCw, Search, Minus, Square, X } from "./icons";

/* Barra de comando — também é a barra de título (janela sem decoração):
   a área livre arrasta a janela (`data-tauri-drag-region`), e os botões à
   direita fazem minimizar / maximizar / fechar. */
export function TopBar() {
  const win = getCurrentWindow();
  return (
    <div className="topbar" data-tauri-drag-region>
      <button className="icobtn" title="Choose music folder" type="button">
        <Folder />
      </button>
      <span className="path">~/Music/Library</span>
      <button className="icobtn" title="Rescan folder" type="button">
        <RefreshCw />
      </button>

      <label className="search">
        <Search />
        <input type="text" placeholder="Search your library" spellCheck={false} />
      </label>

      <div className="wbtns">
        <button className="wbtn" title="Minimize" type="button" onClick={() => win.minimize()}>
          <Minus />
        </button>
        <button
          className="wbtn"
          title="Maximize"
          type="button"
          onClick={() => win.toggleMaximize()}
        >
          <Square />
        </button>
        <button className="wbtn close" title="Close" type="button" onClick={() => win.close()}>
          <X />
        </button>
      </div>
    </div>
  );
}
