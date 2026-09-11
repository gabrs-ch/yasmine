# Arquitetura

Yasmine é um player local com dois frontes — desktop e Android — sobre um
núcleo Rust compartilhado. Este documento descreve como as peças se
encaixam e por que as fronteiras estão onde estão. O recurso de sync tem
documento próprio: [`sync.md`](sync.md).

## Crates

| Caminho | Papel | Linhas |
|---|---|---|
| `crates/core` | Modelo, schema SQLite, scan, consultas, playlists, hash de conteúdo, cache de capa. Compartilhado entre PC e Android. | ~4000 |
| `crates/audio` | Decode e playback no PC: `cpal` + `symphonia`, ring buffer lock-free, gapless, nivelador de volume. **Não vai pro Android.** | ~1600 |
| `crates/sync` | Pareamento, mDNS, canal Noise, protocolo de transferência, merge. Único crate que fala com a rede e único que mexe com cripto. | ~2200 |
| `crates/sync-host` | Binário `yasmine-sync-host`: serve a biblioteca sem abrir a UI. | ~470 |
| `crates/pc-app` | App desktop: back Rust (Tauri 2) + front React/TS em `ui/`. | ~2300 + ~2000 TS |
| `crates/android-ffi` | Ponte uniffi (`YasmineLibrary` + `Syncer`) → Kotlin. | ~660 |
| `android/` | App Android (Compose + ExoPlayer + CameraX). Ver [`android.md`](android.md). | ~2300 Kotlin |
| `tools/libgen` | Gerador de biblioteca sintética para medição. | |

O binário do player chama-se `yasmine`. Os crates internos mantêm o prefixo
`player-*` (`player-core`, `player-audio`, `player-pc`) — identificadores de
implementação, não aparecem pro usuário. Os crates de sync usam `yasmine-*`
porque um deles (`yasmine-sync-host`) *é* um binário que o usuário invoca.

## O que cada fronte substitui

| Função | PC | Android |
|---|---|---|
| Índice, schema, migração | `player-core::db` | mesmo, via FFI |
| Scan de pasta | `player-core::scan` | mesmo, via FFI |
| Consultas, playlists, hash | `player-core::{library,playlist,hash}` | mesmo, via FFI |
| Sync | `yasmine-sync` | mesmo, via FFI |
| **Decode + playback** | `player-audio` | **ExoPlayer** |
| **UI** | Tauri 2 + React/TS | **Compose** |
| Cache de capa | `player-core::art` | mesmo, via FFI |

No Android o ExoPlayer toca porque notificação, tela de bloqueio, Bluetooth
e Android Auto saem prontos e testados dele. O Rust entrega o caminho do
arquivo (`library::playback_info`) e a lista; o Kotlin monta a fila.

**O Rust é o único escritor do arquivo SQLite.** O Kotlin consulta pela FFI
e nunca abre o banco direto. Dois escritores no mesmo arquivo dá corrupção
difícil de rastrear.

## Modelo de dados

Um arquivo SQLite (`library.db`), WAL + `synchronous=NORMAL` + mmap. Schema
em [`schema.sql`](../crates/core/src/schema.sql), versionado em
`PRAGMA user_version` (`SCHEMA_VERSION = 5`); migrações incrementais em
[`db.rs`](../crates/core/src/db.rs), cada passo num braço `if version < N`.
O `schema.sql` descreve só a criação do zero — instalação nova e upgrade
passam pelo mesmo `ALTER TABLE`, sem duplicar a lógica.

O schema se divide em duas camadas com regras de propriedade diferentes. É a
distinção que organiza o sync inteiro.

### Camada derivada — o disco manda

Tudo aqui é cache reconstruível: apagar o banco e reescanear devolve o mesmo
estado.

- `library_root` — pastas apontadas. O caminho da faixa é **relativo** à
  raiz, então mover a pasta inteira de lugar não invalida nada.
- `artist`, `album`, `cover_art` — normalizados. `artist.name_key` é o nome
  dobrado (minúscula, sem acento, espaço colapsado), pra "Legião Urbana" e
  "legiao urbana" não virarem dois.
- `track` — uma linha por arquivo. Guarda `(file_size, mtime_ns)` pro
  fast-path do rescan, metadata das tags, propriedades do stream, e
  `content_hash` (BLAKE3, **`NULL` até alguém precisar**).
- `track_fts` — FTS5 virtual, `tokenize = "unicode61 remove_diacritics 2"`.
  Busca instantânea e insensível a acento numa decisão só.

### Camada do usuário — só existe no banco

Não dá pra reconstruir a partir dos arquivos. É a única camada que precisa
de merge de verdade no sync.

- `device` — um device = uma chave pública estática Noise. `device.id` **é**
  essa chave. `is_self` marca o próprio; `paired_at`, `last_sync_at`.
- `playlist` — `id` é UUIDv7 (ordenável no tempo, gerado offline).
  `updated_at` pro LWW, `origin` (device) pra desempatar, `deleted` como
  túmulo.
- `playlist_item` — chave `(playlist_id, position)`, com `deleted`/
  `deleted_at`. `track_key` = `content_hash`, **não** `track.id`.
- `play_count` — `(track_key, device_id) → count`. G-Counter.
- `track_state` — `track_key → { last_played_at, rating, rating_updated_at,
  resume_pos_ms, resume_updated_at }`. LWW por campo.
- `meta` — chave/valor do próprio app: raiz da biblioteca, volume, chave
  estática do sync, imagens escolhidas. Deliberadamente **não** é tabela de
  configuração do usuário.

### `TrackKey`: a identidade que atravessa devices

`track.id` é autoincrement local — o mesmo arquivo tem id diferente em cada
aparelho. Tudo que precisa sobreviver ao sync referencia a faixa por
`TrackKey` = BLAKE3 do conteúdo do arquivo.

Duas consequências que o Android herda de graça: um `playlist_item` cujo
`track_key` não tem faixa local vira **buraco na lista, não some** (é a faixa
que ainda não chegou); e reeditar metadata muda o hash, então versões
divergentes do mesmo arquivo coexistem como faixas distintas até o usuário
resolver.

### Hash preguiçoso

Hashear é ler o arquivo inteiro. Fazer isso no scan multiplicaria o custo da
primeira varredura por dez sem ninguém ter pedido sync. Então
`track.content_hash` nasce `NULL` e só é preenchido quando alguém precisa:
uma faixa entrando numa playlist, ou o sync comparando bibliotecas.
[`hash::ensure_hashes`](../crates/core/src/hash.rs) calcula em paralelo
(`rayon`) só o que falta e grava numa transação. O índice parcial
`track_by_hash ... WHERE content_hash IS NOT NULL` mantém isso barato.

## Scan incremental

[`scan.rs`](../crates/core/src/scan.rs):

1. Travessia paralela da pasta (`jwalk`) coletando caminho, tamanho, mtime.
2. Divide em "inalterado" / "processar" comparando `(size, mtime)` com o
   índice — bateu, o arquivo nem é aberto. Rescan de 50k faixas intactas é
   um lote de `stat()`.
3. Parse de tag em paralelo (`rayon`, via `lofty`), com a capa já
   deduplicada por hash dentro do worker — o blob morre ali, não acumula na
   RAM.
4. Uma transação escreve tudo.

`keep_only_root` apaga o que não é da pasta atual: "aponte uma pasta" é a
promessa, e o índice reflete isso.

No desktop, um vigia de arquivos (`notify`) dispara rescan sozinho quando
algo muda no disco. O loop de playback consulta `take_change()` a cada tick
em vez de acordar por callback.

## Cache de capa

A mesma arte se repete em toda faixa do álbum, e é o maior consumidor de
memória de um player. Deduplicada por BLAKE3 do blob; o original **nunca**
entra na RAM nem no banco. O que sobra em disco são duas miniaturas JPEG:

```
<cache>/art/<hex[0..2]>/<hex>_96.jpg     linha da lista
<cache>/art/<hex[0..2]>/<hex>_512.jpg    capa em destaque
```

Reamostragem Lanczos3, rodando uma vez por capa dentro do worker paralelo do
scan que já é I/O bound — o custo a mais não aparece no relógio. `cover_art`
guarda só `blob_hash`, `mime` e dimensões.

O desktop serve essas miniaturas ao webview por um protocolo custom
(`art://localhost/<hex>/<tamanho>`); o Android lê o arquivo direto pelo
caminho que `artThumbPath` monta.

## Consultas

A lista **nunca carrega a biblioteca na RAM**. Uma "view" é um
`Vec<TrackId>` ordenado (400 KB pra 50k faixas); a UI pede só as ~40 linhas
visíveis com `rows(db, &ids)`. Nunca `LIMIT/OFFSET`, que é O(n).

`library.rs` expõe `view`, `search`, `by_artist`, `artists`, `rows`,
`playback_info`, `stats`, `find_by_absolute_path`.

## Desktop: processo e fronteiras

`crates/pc-app` é um app Tauri 2. O Rust é o back; a UI é React/TS servida
pelo webview do SO (WebKitGTK no Linux, WebView2 no Windows).

- `AppState` atrás de `Mutex`, como estado gerenciado do Tauri: banco,
  `Engine` de áudio, fila, view atual, vigia de arquivos.
- `commands.rs` — a ponte IPC. Cada comando trava o estado, chama
  `player-core`, devolve DTO. Sem lógica de UI.
- `playback.rs` — thread que bombeia os eventos do motor de áudio e
  transmite `playback://state` a cada 250 ms.
- `scan.rs` — scan em thread com sua própria conexão SQLite (WAL cuida da
  concorrência), emitindo `scan://progress` e `scan://done`.
- `sync_host.rs` — o servidor de sync embutido, em estado gerenciado
  **à parte** do `AppState`: ele roda em threads próprias e não deve prender
  o lock que o loop de playback pega 4×/s.

## Onde os dados moram

`directories::ProjectDirs::from("", "", "Yasmine")` → `data_dir()/library.db`
e `cache_dir()/` pras miniaturas. No Android o equivalente é o diretório
privado do app, e a FFI **recebe** esses caminhos do Kotlin em vez de
descobrir sozinha.

O app desktop e o `yasmine-sync-host` resolvem o mesmo caminho e abrem o
mesmo banco — por isso os dois têm que concordar no schema.

## Convenções

Rust estável, edição 2024, `rust-version = 1.98`. `unsafe_code = "deny"` no
workspace. CI roda `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings` e `cargo test --workspace` a cada push.

Comentário explica o **porquê**, não o quê. Português no código e na
documentação; as strings de interface são em inglês nos dois apps.

Nenhuma otimização vira configuração exposta: não existe "modo performance",
ajuste de buffer nem limpeza manual de cache.
