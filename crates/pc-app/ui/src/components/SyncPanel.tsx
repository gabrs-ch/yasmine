import { useStore } from "../store";
import { X } from "./icons";

/* Painel de "Sincronizar com o celular": mostra o QR que o app Android lê
   pra parear na LAN e baixar a biblioteca. O servidor sobe quando o painel
   abre (store.openSync) e cai quando fecha (store.closeSync). */
export function SyncPanel() {
  const sync = useStore((s) => s.sync);
  const closeSync = useStore((s) => s.closeSync);

  const phase = sync?.phase ?? "starting";
  const detail = sync?.detail ?? null;

  const status: Record<string, string> = {
    starting: "Subindo o servidor…",
    waiting: "Aguardando o celular ler o QR…",
    connected: detail ? `${detail} conectou` : "Celular conectou",
    sending: `Enviando a biblioteca… ${detail ?? ""}`,
    done: "Concluído — o celular tem a biblioteca ✓",
    error: detail ?? "Falhou",
  };

  return (
    <div className="sync-overlay" onPointerDown={() => void closeSync()}>
      <div className="sync-card" onPointerDown={(e) => e.stopPropagation()}>
        <button className="sync-close" type="button" title="Fechar" onClick={() => void closeSync()}>
          <X />
        </button>

        <div className="sync-title">Sincronizar com o celular</div>
        <div className="sync-sub">
          No Yasmine do celular: <b>Parear</b> → aponte a câmera pro código.
        </div>

        <div className="sync-qr">
          {sync?.qrSvg ? (
            <div dangerouslySetInnerHTML={{ __html: sync.qrSvg }} />
          ) : (
            <div className="sync-qr-empty" />
          )}
        </div>

        <div className={`sync-phase${phase === "error" ? " err" : ""}`}>{status[phase]}</div>

        {sync?.pairUrl && (
          <div className="sync-url" title="Endereço de pareamento (caso a câmera não leia)">
            {sync.pairUrl}
          </div>
        )}
      </div>
    </div>
  );
}
