# Contexto pro app Android e o sync local

Documento de handoff. Descreve o que o Yasmine é, como o lado PC foi
construído, e o que o app Android precisa fazer — com foco no recurso que
motiva ele existir: **parear PC e celular na mesma rede local por QR code e
baixar pro celular as músicas que estão na biblioteca do computador.**

Nada aqui é código novo. É o mapa pra quem for escrever o Android sem ter
acompanhado o desenvolvimento do PC.

---

## 1. O que é o Yasmine

Player de música local. O usuário aponta uma pasta; o app varre, indexa e
toca. Sem conta, sem nuvem, sem servidor. Para sincronizar entre dois
aparelhos, aponta um pro outro — a rede local resolve o resto.

Três princípios, e eles explicam quase toda decisão do código:

1. **Otimização é o critério de desempate** em toda decisão técnica.
2. **Nenhuma otimização vira configuração.** Não existe "modo performance",
   ajuste de buffer, nem limpeza manual de cache. O default é o melhor que
   dá, e é invisível.
3. Cada fase entrega algo usável.

Estado hoje: **Fases 0 a 2 completas** — o player de PC funciona (pasta →
scan → índice → tocar; lista, transporte, busca, seek, fila,
shuffle/repeat, playlists, modo compacto, vigia de arquivos, nivelador de
volume, "ver só um artista", "abrir com"). **Fase 3 (Android standalone) e
Fase 4 (sync na LAN) não começaram.** `crates/sync` e `crates/android-ffi`
são stubs — só o doc-comment com o desenho decidido.

---

## 2. Arquitetura em crates

Workspace Cargo. O que atravessa a fronteira pro Android é só o `core`.

| Crate | Papel | Vai pro Android? |
|---|---|---|
| `crates/core` | Modelo, schema SQLite, indexação, consultas da biblioteca, playlists, hash de conteúdo. | **Sim** — é o único crate compartilhado. |
| `crates/audio` | Decode e playback no PC (`cpal` + `symphonia`, ring buffer lock-free, gapless). | **Não** — no Android quem toca é o ExoPlayer. |
| `crates/sync` | Pareamento, descoberta mDNS, canal Noise, diff e transferência. | **Sim** (quando existir). É o único crate que fala com a rede e o único que mexe com cripto. |
| `crates/pc-app` | App desktop (`egui`/`eframe`, backend `glow`). | Não. O equivalente Android é Compose. |
| `crates/android-ffi` | Bindings Kotlin via uniffi, expondo `core` (e depois `sync`). | É a ponte. |
| `tools/libgen` | Gerador de biblioteca sintética pra medição. | Não. |

O binário final chama-se `yasmine`; os crates internos mantêm o prefixo
`player-*` — identificadores de implementação, não aparecem pro usuário.

Toolchain: Rust **stable** (`rust-version = 1.98`, edição 2024). CI roda
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`
e `cargo test --workspace` a cada push. `unsafe_code = "deny"` no workspace.

---

## 3. Modelo de dados — a parte que importa pro sync

Um arquivo SQLite (`library.db`), aberto com WAL + `synchronous=NORMAL` +
`mmap`. Schema em [`crates/core/src/schema.sql`](../crates/core/src/schema.sql),
versionado em `PRAGMA user_version` (`SCHEMA_VERSION = 3` hoje); migrações
incrementais em [`db.rs`](../crates/core/src/db.rs), cada passo num braço
`if version < N`.

**Regra de ouro do arquivo SQLite: o Rust é o único escritor.** O Kotlin
consulta pela FFI, nunca abre o SQLite direto. Dois escritores no mesmo
arquivo dá corrupção difícil de rastrear.

### Duas camadas, regras de propriedade diferentes

**[A] Camada derivada** — os arquivos em disco são a fonte da verdade.
Tudo aqui é cache reconstruível: apagar o DB e reescanear devolve o mesmo
estado.

- `library_root` — pastas apontadas. Caminho de faixa é relativo à raiz, então
  mover a pasta inteira não invalida nada.
- `artist`, `album`, `cover_art` — normalizados. `artist.name_key` é o nome
  dobrado (minúscula, sem acento, espaço colapsado) pra "Legião Urbana" e
  "legiao urbana" não virarem dois.
- `track` — uma linha por arquivo. Guarda `(file_size, mtime_ns)` pro
  fast-path do rescan, metadata das tags, propriedades do stream
  (`sample_rate` etc.), e `content_hash` (BLAKE3, **`NULL` até alguém
  precisar**).
- `track_fts` — FTS5 virtual, `tokenize = "unicode61 remove_diacritics 2"`.
  Busca instantânea e insensível a acento numa decisão só.

**[B] Camada do usuário** — só existe no DB, não dá pra reconstruir.
Playlists, contagem de plays, rating, posição de retomada. **É a única
camada que precisa de merge de verdade no sync.**

- `device` — um device = uma chave pública estática Noise. `device.id` **é**
  essa chave (32 bytes). Parear e identificar são a mesma operação.
  `is_self` marca o próprio; `paired_at`, `last_sync_at`.
- `playlist` — `id` é UUIDv7 (ordenável no tempo, gerado offline).
  `updated_at` pro LWW, `origin` (device) pra desempatar `updated_at` igual,
  `deleted` como **tombstone** (sem ele, o outro device reintroduz a
  playlist apagada no próximo encontro).
- `playlist_item` — chave `(playlist_id, position)`. `position` é uma
  **string de índice fracionário** (`"a0"`, `"a0V"`, `"a1"` — ver
  [`fracidx.rs`](../crates/core/src/fracidx.rs)): inserir/mover escreve UMA
  linha em vez de renumerar a playlist inteira, e dois devices reordenando
  ao mesmo tempo fazem merge sem colidir. **`track_key` = `content_hash`, não
  `track.id`.**
- `play_count` — `(track_key, device_id) → count`. **G-Counter.** LWW aqui
  estaria errado: se o PC tocou 3× e o celular 2×, LWW guarda 3 e perde 2.
  Total = `SUM(count)`, merge = `MAX(count)` por par. Converge sem perder
  play nenhum.
- `track_state` — `track_key → { last_played_at, rating, rating_updated_at,
  resume_pos_ms, resume_updated_at }`. **LWW por campo:** dá pra mudar o
  rating num device e a posição de retomada no outro sem um sobrescrever o
  outro.
- `meta` — chave/valor do próprio app (`device_id`, última pasta, volume).
  Deliberadamente **não** é tabela de configuração do usuário.

### A identidade que atravessa devices: `TrackKey`

`track.id` é autoincrement local — **o mesmo arquivo tem id diferente em
cada aparelho.** Então tudo que precisa sobreviver ao sync referencia a
faixa por `TrackKey` = **BLAKE3 do conteúdo do arquivo** (32 bytes, cobre os
bytes do arquivo incluindo as tags).

Consequências que o Android herda de graça:

- Um `playlist_item` cujo `track_key` não tem faixa local vira **buraco na
  lista, não some** — é a faixa que ainda não chegou por sync.
- Reeditar metadata muda o hash. A playlist guarda o hash conhecido no
  momento; versões divergentes do mesmo arquivo coexistem como faixas
  distintas até o usuário resolver. Aceitável — o sync reconcilia por
  conteúdo, não por "nome parecido".

### Hash preguiçoso

Hashear é ler o arquivo inteiro. Fazer isso no scan multiplicaria o custo da
primeira varredura por dez sem ninguém ter pedido sync. Então
`track.content_hash` nasce `NULL` e só é preenchido quando alguém precisa:
uma faixa entrando numa playlist, ou **o sync comparando bibliotecas.**
[`hash::ensure_hashes(db, &[TrackId])`](../crates/core/src/hash.rs) calcula
em paralelo (`rayon`) só o que falta e grava numa transação. Índice parcial
`track_by_hash ... WHERE content_hash IS NOT NULL` mantém isso barato.

O diff do sync **compara `(file_size, mtime_ns)` primeiro** e só força o
BLAKE3 quando diverge — hashear a biblioteca inteira a cada sync seriam
minutos de I/O por nada.

---

## 4. Como o lado PC foi construído (pra dar contexto)

Nada disso é obrigação do Android replicar — é o que o `core` já resolve, e
o que dá pra reaproveitar.

### Scan incremental ([`scan.rs`](../crates/core/src/scan.rs))

1. Travessia paralela da pasta (`jwalk`) coletando caminho, tamanho, mtime.
2. Divide em "inalterado" / "processar" comparando `(size, mtime)` com o
   índice — bateu, o arquivo nem é aberto. **Rescan de 50k faixas intactas =
   um lote de `stat()`, ~0,2 s.**
3. Parse de tag em paralelo (`rayon`, via `lofty`), com a capa já
   deduplicada por hash dentro do worker — o blob morre ali, não acumula na
   RAM.
4. Uma transação escreve tudo.

`scan(db, root, art) -> ScanReport` e `scan_with_progress(..., &AtomicUsize)`
pra UI mostrar andamento. `keep_only_root(db, root)` apaga o que não é da
pasta atual — "aponte uma pasta" é a promessa, o índice reflete isso.

### Cache de capa ([`art.rs`](../crates/core/src/art.rs))

A mesma arte se repete em toda faixa do álbum. Deduplicada por BLAKE3 do
blob. O original **nunca entra na RAM nem no DB** — o que sobra em disco são
duas miniaturas JPEG por capa:

```
<cache>/art/<hex[0..2]>/<hex>_96.jpg     linha da lista
<cache>/art/<hex[0..2]>/<hex>_512.jpg    capa em destaque
```

Reamostragem Lanczos3 (roda uma vez por capa, no worker paralelo do scan que
já é I/O bound). `cover_art` no DB guarda só `blob_hash`, `mime`, dimensões.

### Consultas ([`library.rs`](../crates/core/src/library.rs))

A lista **nunca carrega a biblioteca na RAM.** Uma "view" é um
`Vec<TrackId>` ordenado (400 KB pra 50k faixas); a UI pede só as ~40 linhas
visíveis com `rows(db, &ids)`. Nunca `LIMIT/OFFSET` (é O(n)). API:
`view`, `search`, `by_artist`, `rows`, `playback_info`, `stats`,
`find_by_absolute_path` (resolve caminho → `TrackId`, pro "abrir com").

### Áudio no PC ([`crates/audio`](../crates/audio)) — o Android **não** usa

`cpal` + `symphonia` direto, sem `rodio`, pra abrir o device na taxa do
arquivo (zero reamostragem) e ter gapless de graça pré-decodificando no
mesmo ring buffer. Três threads: worker (decodifica), callback de áudio
(só copia bytes, tempo real, sem alocar/lock/I/O), UI (manda comando, lê
átomos). **No Android quem faz tudo isso é o ExoPlayer** — notificação, tela
de bloqueio, Bluetooth e Android Auto saem prontos e testados.

### Onde os dados moram

`directories::ProjectDirs::from("", "", "Yasmine")` →
`data_dir()/library.db` e `cache_dir()/` pras miniaturas. No Android o
equivalente é o diretório privado do app; a FFI deve receber esses caminhos
do lado Kotlin, não descobrir sozinha.

### Distribuição

Release por tag `v*` no GitHub Actions builda binário pra Linux e Windows e
publica como release. `packaging/` tem o `.desktop` + `install.sh` (Linux) e
o script de associação de tipos de arquivo (Windows).

---

## 5. O que o Android herda e o que ele substitui

| Função | PC | Android |
|---|---|---|
| Índice, schema, migração | `player-core::db` | mesmo, via FFI |
| Scan de pasta | `player-core::scan` | mesmo, via FFI |
| Consultas da biblioteca | `player-core::library` | mesmo, via FFI |
| Playlists | `player-core::playlist` | mesmo, via FFI |
| Hash de conteúdo | `player-core::hash` | mesmo, via FFI |
| Sync | `player-sync` (a fazer) | mesmo, via FFI |
| **Decode + playback** | `player-audio` | **ExoPlayer (Kotlin)** |
| **UI** | `egui`/`eframe` | **Compose (Kotlin)** |
| Cache de capa | `player-core::art` | mesmo, via FFI (ou regenera das tags) |

O ExoPlayer lê os arquivos direto do disco. O Rust dá pra ele o caminho
(`playback_info(db, id) -> PlaybackInfo { path, gain_db, peak }`) e a lista
de faixas; o Kotlin monta a fila e toca. O `gain_db`/`peak` do nivelador
podem virar `volume` / `PlaybackParameters` no ExoPlayer, ou serem
ignorados na v1.

---

## 6. Sync local — o recurso a construir (Fase 4)

**Objetivo concreto:** dois aparelhos na mesma LAN. O celular lê um QR code
do PC, os dois passam a se conhecer, e o celular **baixa os arquivos de
áudio da biblioteca do PC** (mais as playlists / plays / rating, que fazem
merge). Depois disso o celular tem uma biblioteca local própria — dá pra
desconectar e continuar ouvindo.

Desenho decidido (nos doc-comments de
[`crates/sync/src/lib.rs`](../crates/sync/src/lib.rs) e
[`schema.sql`](../crates/core/src/schema.sql)); o que falta é implementar e
fechar os pontos em aberto listados no fim.

### 6.1 Pareamento por QR code

O QR carrega a **chave pública estática Noise do PC** (32 bytes, base64/hex).
Essa chave **é** o `DeviceId` do PC — não tem passo separado de
"identificar".

- O celular lê o QR, grava o PC em `device` (`is_self = 0`, `paired_at`).
- Pareamento é **bidirecional**: o celular também tem a sua chave estática
  (gerada no primeiro boot, guardada em `meta`), e o PC precisa dela pra
  autenticar de volta. Duas formas de fechar isso: (a) o celular mostra o
  próprio QR pro PC ler, ou (b) o celular manda a chave dele pelo primeiro
  handshake e o PC confirma na tela ("parear com <nome>?"). (b) é menos
  fricção; decidir na implementação.
- O QR pode carregar **só a chave** (e o mDNS acha host:porta) ou
  **chave + host:porta** (dispensa mDNS pro primeiro contato, útil quando o
  mDNS está bloqueado na rede). Provável: os dois — chave sempre, host:porta
  opcional.

### 6.2 Descoberta

mDNS na LAN. O PC publica um serviço (`_yasmine._tcp` ou similar — nome a
definir) com `{DeviceId, host, porta}`. O celular resolve pelo `DeviceId`
que já tem do pareamento. **IP digitado à mão é o plano B** (rede sem mDNS).

### 6.3 Canal

Noise (`snow`), sobre TCP. Como as duas chaves estáticas já são conhecidas
depois do pareamento, o padrão natural é **`Noise_KK`** (autenticação mútua
sem PKI, sem troca de chave no handshake). Confirmar na implementação.
Rejeitar conexão de `DeviceId` não pareado.

### 6.4 Merge — duas camadas, dois algoritmos

**Camada derivada (áudio + capa): endereçada por conteúdo, sem conflito.**

O protocolo é literalmente "tenho / não tenho este hash":

1. Celular manda o conjunto de `content_hash` que já tem (ou, mais barato, um
   resumo — filtro de Bloom / árvore de Merkle sobre os hashes ordenados).
2. PC responde com a metadata das faixas que o celular **não** tem: `{ hash,
   tags, propriedades do stream, hash da capa }`. A metadata vai primeiro pra
   biblioteca do celular aparecer antes dos arquivos terminarem de chegar.
3. Transferência dos blobs de áudio, por hash. O celular grava numa pasta
   local (ex.: `<app>/Musica/`), verifica o BLAKE3 ao terminar cada arquivo,
   e essa pasta vira um `library_root` no índice dele.
4. Capa: ou viaja como blob por `blob_hash` (mesma ideia), ou o celular
   **regenera as miniaturas localmente** a partir das tags dos arquivos
   baixados — `ArtCache` já faz exatamente isso no scan, então é quase de
   graça e evita transferir imagem.

O celular roda `scan` na pasta baixada como se o usuário tivesse apontado
ela. As playlists / plays / rating que vieram no sync **encaixam
automaticamente** porque apontam por `track_key`.

Escopo do "o que baixar": ou **a biblioteca inteira do PC**, ou **só o que
está referenciado pela camada do usuário** (faixas em playlist), ou
**seleção do usuário**. Decisão de UX — a v1 mais simples é "biblioteca
inteira", com barra de progresso e possibilidade de cancelar.

**Camada do usuário: merge de verdade.**

- `playlist`: LWW por `updated_at`, desempate por `origin`. Respeitar o
  tombstone `deleted` — playlist apagada de um lado fica apagada.
- `playlist_item`: **união** dos itens das duas pontas. A chave é
  `(playlist_id, position)` com índice fracionário, então não tem
  renumeração; item que só existe de um lado entra. Item removido de um lado
  precisa de tombstone também (hoje `playlist_item` não tem coluna
  `deleted` — **isso é uma lacuna do schema pra Fase 4**, ver pontos em
  aberto).
- `play_count`: G-Counter. Pra cada `(track_key, device_id)`, fica o
  `MAX(count)` das duas pontas. Total continua sendo `SUM`.
- `track_state`: LWW **por campo**. `rating` vence pelo maior
  `rating_updated_at`; `resume_pos_ms` pelo maior `resume_updated_at`;
  independentes.

### 6.5 Diff eficiente

- Nunca hashear a biblioteca toda: comparar `(file_size, mtime_ns)` e forçar
  `ensure_hashes` só nas faixas que entram na comparação.
- Resumo dos hashes (Bloom/Merkle) pra não mandar 50k × 32 bytes a cada
  sync.
- Transferência retomável: guardar o que já veio, continuar de onde parou.
- Ordem: `device` handshake → camada do usuário (pequena, rápida) →
  metadata das faixas faltantes → blobs de áudio (o grosso).

### 6.6 O que a FFI precisa expor (uniffi)

O `player-android-ffi` vai crescer pra cobrir, além do que já dá contexto:

- Abrir/migrar o `Db` recebendo o caminho do Kotlin.
- `scan` / `scan_with_progress`, `keep_only_root`.
- `library::{view, search, by_artist, rows, playback_info, stats}`.
- `playlist::*`, `hash::ensure_hashes`.
- **A API de `player-sync`**: gerar/ler o payload do QR, iniciar descoberta,
  conectar a um `DeviceId`, rodar o sync com callback de progresso, listar
  devices pareados, desparear.

Kotlin nunca abre o SQLite direto. Threading: as chamadas de scan/sync são
longas — expor como suspensas / com callback, rodando fora da main thread.

---

## 7. Pontos em aberto (decidir na Fase 4)

- **Tombstone de `playlist_item`.** O schema hoje não guarda "item removido".
  Sem isso, um item apagado de um lado volta no sync. Precisa de coluna
  `deleted` + `deleted_at` (ou uma tabela de tombstones), migração
  `SCHEMA_VERSION 4`.
- **Padrão Noise exato** (`KK` é o palpite) e **service name mDNS**.
- **Porta** — fixa, ou negociada, ou range com fallback.
- **QR carrega host:porta ou só a chave?**
- **Resumo de hashes**: Bloom filter (simples, com falso-positivo aceitável)
  vs árvore de Merkle (exato, mais código).
- **Escopo do download**: biblioteca inteira / só o referenciado / seleção.
- **Capa**: transferir blob vs regenerar das tags no celular (preferência:
  regenerar).
- **Faixa com hash divergente** (metadata editada nos dois lados): hoje
  coexistem como faixas distintas. Confirmar que isso é aceitável ou desenhar
  reconciliação.
- **Sync incremental vs full**: `device.last_sync_at` existe pra isso —
  definir o que "desde a última vez" significa em cada camada.

---

## 8. Convenções do repositório

- Rust **stable**, edição 2024. `cargo fmt` obrigatório (CI barra), clippy
  com `-D warnings`, `unsafe_code` proibido.
- Comentário explica o **porquê**, não o quê. Densidade alta, sem enrolação.
- **Nada de configuração exposta pro usuário.** Buffer, cache, filtro — é
  tudo decisão do código.
- Português no código e na doc.
- Release: tag `v*` → binários Windows/Linux no GitHub Actions.
- Mensagem de commit: assunto curto no imperativo, corpo explicando a
  decisão. Termina com `Co-Authored-By:` quando cabível.
