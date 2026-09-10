import { useEffect } from "react";
import { TopBar } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { MainPane } from "./components/MainPane";
import { PlayerBar } from "./components/PlayerBar";
import { EmptyState } from "./components/EmptyState";
import { ScanToast } from "./components/ScanToast";
import { useStore } from "./store";

export function App() {
  const ready = useStore((s) => s.ready);
  const root = useStore((s) => s.root);
  const init = useStore((s) => s.init);

  useEffect(() => {
    void init();
  }, [init]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) {
        return;
      }
      const s = useStore.getState();
      switch (e.key) {
        case " ":
          e.preventDefault();
          void s.playPause();
          break;
        case "ArrowRight":
          void s.seekBy(5000);
          break;
        case "ArrowLeft":
          void s.seekBy(-5000);
          break;
        case "s":
        case "S":
          void s.toggleShuffle();
          break;
        case "r":
        case "R":
          void s.cycleRepeat();
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="win">
      <TopBar />
      <div className="body">
        <Sidebar />
        {ready && !root ? <EmptyState /> : <MainPane />}
      </div>
      <PlayerBar />
      <ScanToast />
    </div>
  );
}
