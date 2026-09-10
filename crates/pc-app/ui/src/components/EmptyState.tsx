import { LibraryBig } from "./icons";
import { useStore } from "../store";

export function EmptyState() {
  const pickFolder = useStore((s) => s.pickFolder);
  return (
    <section className="card main empty">
      <div className="empty-inner">
        <div className="empty-mark">
          <LibraryBig />
        </div>
        <p className="empty-text">Choose a music folder to get started.</p>
        <button className="empty-btn" type="button" onClick={() => void pickFolder()}>
          Choose music folder…
        </button>
      </div>
    </section>
  );
}
