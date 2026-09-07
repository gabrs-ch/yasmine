# Player

Player de música para PC (Rust) e Android, com sync direto por LAN — sem
conta, sem nuvem, sem servidor. O usuário aponta uma pasta; para sincronizar,
aponta o outro device.

*(nome e identidade visual — roxo/azul/preto — a definir)*

## Princípios

1. **Otimização é o critério de desempate** em toda decisão técnica.
2. **Nenhuma otimização vira configuração.** Não existe "modo performance",
   ajuste de buffer nem limpeza manual de cache. O default é o melhor que a
   gente consegue, e é invisível.
3. Cada fase entrega algo usável.

## Crates

| Crate | Papel |
|---|---|
| `crates/core` | Modelo, schema, índice. Único crate compartilhado entre PC e Android. |
| `crates/audio` | Decode e playback no PC (`cpal` + `symphonia`). Não vai pro Android. |
| `crates/sync` | Pareamento, mDNS, canal Noise, sync. Isolado: é o único que fala com a rede. |
| `crates/pc-app` | App desktop (`egui`/`eframe`). |
| `crates/android-ffi` | Bindings Kotlin via uniffi. |
| `tools/libgen` | Gerador de biblioteca sintética para medição. |

## Decisões fechadas

**`egui`/`eframe` com backend `glow`, não `wgpu`.** Criar o contexto GL é mais
rápido e traz menos dependência — aparece direto no cold start. Vêm junto:
repaint reativo (só em evento) e, tocando, repaint limitado a ~4 Hz. O padrão
de redesenhar a 60 fps é metade do custo de CPU de um player parado.

**`cpal` + `symphonia` direto, sem `rodio`.** O rodio reamostra sempre que a
taxa do device não bate com a do arquivo, com interpolação linear. Indo direto
no cpal dá pra abrir o device *na taxa do arquivo* e eliminar a reamostragem.
E o gapless sai quase de graça pré-decodificando no mesmo ring buffer.

**SQLite, definitivo.** WAL + `synchronous=NORMAL` + mmap. 50k faixas não chega
perto de ser gargalo; o custo real está no I/O do scan.

**FTS5 com `remove_diacritics 2`.** Uma decisão resolve busca instantânea *e*
acento: "jose" acha "José" sem coluna normalizada extra.

**A lista nunca carrega a biblioteca na RAM.** A view é um `Vec<TrackId>`
ordenado (400 KB para 50k faixas) e a UI busca só as linhas visíveis — nunca
`LIMIT/OFFSET`, que é O(n).

**Capa deduplicada por BLAKE3, miniaturas em disco.** É o maior consumidor de
memória de um player: a mesma arte se repete em toda faixa do álbum. Original
nunca entra na RAM.

**Arquivo manda na metadata; o DB é cache derivado.** Estado do usuário
(playlist, plays, rating) vive à parte, com timestamp por campo. É o que
barateia o sync: áudio vira transferência endereçada por conteúdo, sem
conflito possível, e só o estado do usuário precisa de merge — LWW por campo,
e `MAX` por (faixa, device) para contagem de plays, que somada não perde play
nenhum.

**No Android o ExoPlayer toca e o Rust indexa.** Notificação, tela de bloqueio,
Bluetooth e Android Auto saem prontos e testados. O Rust é o **único** escritor
do arquivo SQLite; o Kotlin só consulta via FFI.

## Fases

- [x] **0 — Fundamentos.** Workspace, schema, clippy/fmt no CI, gerador de biblioteca.
- [ ] **1 — Player PC.** Pasta → scan → índice → tocar. Lista, play/pause/next/prev, busca.
- [ ] **2 — Polimento PC.** Fila, playlists, shuffle/repeat, atalhos, tray, `notify`, profiling.
- [ ] **3 — Android standalone.** Compose + `core` via uniffi.
- [ ] **4 — Sync na LAN.** QR → mDNS → Noise → diff por hash.
- [ ] **5 — Refinamento.** Profiling real, biblioteca grande, onboarding.

## Desenvolvimento

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Biblioteca sintética para medir scan, índice e memória — determinística, a
mesma seed dá o mesmo corpus byte a byte:

```bash
cargo run --release -p libgen -- --out ./testdata/lib50k --tracks 50000
```

50k faixas em ~2,5s, ocupando 1,2 GB. Como regenerar é barato, o corpus grande
não fica versionado nem guardado — `testdata/` está no `.gitignore`.
