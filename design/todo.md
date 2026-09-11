# Pendências

Backlog curto do que ficou em aberto. O que é grande o bastante pra virar
fase está no README; aqui é o resto.

## Desktop

- **Botão "check for updates".** Plugin updater do Tauri: par de chaves de
  assinatura (o usuário gera), `latest.json` por release no CI, botão →
  baixa e instala. ~1 h.

## Android

- **Build release assinado.** Hoje o APK da release é debug, assinado com a
  chave de debug — instala, mas o Android avisa origem desconhecida. Um
  release de verdade precisa de keystore em Secrets.
- **Música sobrevivendo à desinstalação.** Hoje fica em armazenamento
  privado do app (`getExternalFilesDir`), que o Android apaga junto. Mover
  pra `MediaStore`/`/sdcard/Music` faria sobreviver — ao custo de permissão
  de storage e de aparecer em outros players.

## Sync

Os limites conhecidos da v1 estão em [`docs/sync.md`](../docs/sync.md#o-que-ficou-pra-depois):
sentido inverso (celular → PC), diff por `(size, mtime)`, sync incremental,
resumo de hashes, escopo seletivo.
