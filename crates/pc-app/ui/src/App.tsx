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
