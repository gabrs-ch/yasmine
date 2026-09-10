import { TopBar } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { MainPane } from "./components/MainPane";
import { PlayerBar } from "./components/PlayerBar";

export function App() {
  return (
    <div className="win">
      <TopBar />
      <div className="body">
        <Sidebar />
        <MainPane />
      </div>
      <PlayerBar />
    </div>
  );
}
