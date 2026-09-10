import { useStore } from "../store";

/* Aviso passageiro no canto inferior — o balãozinho flutuante, não uma faixa
   de largura inteira. Mostra o andamento do scan ou o resumo do fim. */
export function ScanToast() {
  const scan = useStore((s) => s.scan);
  const toast = useStore((s) => s.toast);
  if (!scan && !toast) return null;
  return (
    <div className="scan-toast">
      {scan
        ? `Scanning… ${scan.done.toLocaleString()} files`
        : toast}
    </div>
  );
}
