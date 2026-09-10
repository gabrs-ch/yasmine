# Testar o sync PC ↔ Android ponta a ponta

Pré-requisitos: PC e celular na **mesma rede WiFi**. O desktop Yasmine já
migrado pra schema **v5** (release `v0.5.6` ou `git pull` do `main`).

## 1. Lado PC — subir o host

Da raiz deste repo:

```sh
# CLI: imprime o QR no terminal (blocos unicode). Não precisa de lib extra.
cargo run -p yasmine-sync-host -- --mdns

# ou, com janela e QR grande (precisa: libxkbcommon-dev libgl1-mesa-dev):
cargo run -p yasmine-sync-host --features gui -- --gui --mdns
```

Ele abre a **mesma** `~/.local/share/yasmine/library.db` que o app Yasmine
usa, migra pra v5 se preciso, cria a identidade Noise (`meta.sync_sk`) e
serve a biblioteca. O QR carrega `k` (chave = DeviceId), `h`/`p` (IP:porta
da LAN) e `n` (nome).

**No primeiro teste, feche o app Yasmine desktop** — dois processos
escrevendo a mesma `library.db` é seguro sob WAL, mas comece simples. Se
a biblioteca ainda não foi indexada por lá: `--music ~/Musica`.

## 2. Lado celular

Instalar `dist/yasmine-debug-arm64.apk` (arm64, debug). Abrir → aba
**Parear** → permitir a câmera → apontar pro QR → **Baixar**.

Acompanha o progresso (conectando → juntando playlists → comparando
bibliotecas → baixando faixas → indexando). Dá pra **Cancelar**: o que já
veio fica em `.part`, a próxima tentativa retoma.

No fim, a aba **Biblioteca** mostra as faixas (tocam pelo ExoPlayer:
notificação, tela de bloqueio, Bluetooth) e as **Playlists** povoadas. As
capas são regeneradas das tags no próprio celular.

## 3. Conferir o merge (2ª rodada)

1. No celular: dar um rating numa faixa, criar/renomear/reordenar uma
   playlist, apagar um item.
2. Sincronizar de novo com o mesmo PC.
3. Conferir que:
   - rating e posição de retomada não se sobrescreveram (LWW por campo);
   - o item apagado **não** voltou (túmulo v5);
   - contagem de plays somou, não substituiu (G-counter).

## 4. Conferir o desktop depois

Reabrir o app Yasmine → a `library.db` continua legível (v5), as
playlists intactas. Apagar um item de playlist na UI → some da lista e
`SELECT count(*) FROM playlist_item WHERE deleted = 1` sobe.

## Arestas conhecidas

- O app desktop cacheia o estado em memória: pra ver o que veio no sync,
  **reabrir** (ou trocar de fonte e voltar).
- `SQLITE_BUSY` se o desktop estiver aberto e escrevendo durante o sync —
  `busy_timeout = 5000` deve segurar; se der erro, feche o desktop e
  repita.
- IP manual (rede sem mDNS): a tela Parear tem "Digitar IP manualmente"
  (device id hex do QR + `ip:porta`).
