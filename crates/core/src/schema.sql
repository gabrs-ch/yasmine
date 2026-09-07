-- =============================================================================
-- Schema do player. Duas camadas com regras de propriedade DIFERENTES:
--
--   [A] CAMADA DERIVADA  — os arquivos em disco são a fonte da verdade.
--       Tudo aqui é cache reconstruível: apagar o DB e reescanear devolve o
--       mesmo estado. No sync isso vira transferência endereçada por conteúdo
--       (BLAKE3): não existe conflito, só "tenho/não tenho este hash".
--
--   [B] CAMADA DO USUÁRIO — só existe no DB, não tem como reconstruir.
--       Playlists, contagem de plays, rating, posição. É a ÚNICA camada que
--       precisa de merge de verdade no sync. Toda linha aqui referencia faixa
--       por `track_key` (= content_hash), nunca por `track.id`: id é
--       autoincrement local, o mesmo arquivo tem id diferente em cada device.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- [A] Camada derivada
-- -----------------------------------------------------------------------------

-- Pastas que o usuário apontou. Caminhos das faixas são relativos a estas,
-- então mover a pasta inteira de lugar não invalida a biblioteca.
CREATE TABLE library_root (
    id       INTEGER PRIMARY KEY,
    path     TEXT    NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
) STRICT;

CREATE TABLE artist (
    id       INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    -- Nome dobrado (minúscula, sem acento, espaços colapsados). Evita que
    -- "Legião Urbana" e "legiao urbana" virem dois artistas.
    name_key TEXT NOT NULL UNIQUE
) STRICT;

-- Capa deduplicada pelo hash do blob embutido. A mesma arte se repete em toda
-- faixa do álbum: sem dedupe são ~15 GB numa biblioteca de 50k, com dedupe são
-- ~3k álbuns. Os bytes NÃO ficam no DB — miniaturas prontas vão pro cache em
-- disco, em <cache>/art/<hex[0..2]>/<hex>_{96,512}.webp. Original nunca entra
-- na RAM.
CREATE TABLE cover_art (
    id        INTEGER PRIMARY KEY,
    blob_hash BLOB NOT NULL UNIQUE,  -- BLAKE3 (32 bytes) do blob original
    mime      TEXT,
    width     INTEGER,
    height    INTEGER
) STRICT;

CREATE TABLE album (
    id              INTEGER PRIMARY KEY,
    title           TEXT NOT NULL,
    album_artist_id INTEGER REFERENCES artist(id),
    year            INTEGER,
    art_id          INTEGER REFERENCES cover_art(id),
    -- norm(album_artist) || U+001F || norm(title). Separa dois "Greatest Hits"
    -- de artistas diferentes sem precisar de índice composto nullable.
    key             TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE track (
    id       INTEGER PRIMARY KEY,
    root_id  INTEGER NOT NULL REFERENCES library_root(id) ON DELETE CASCADE,
    rel_path TEXT    NOT NULL,

    -- Fast-path do rescan incremental: se (size, mtime_ns) bate, o arquivo não
    -- mudou e a gente nem abre pra ler tag. Rescan de 50k faixas intactas vira
    -- um lote de stat().
    file_size    INTEGER NOT NULL,
    mtime_ns     INTEGER NOT NULL,
    -- BLAKE3 do conteúdo. NULL até alguém precisar (sync ou entrar em
    -- playlist): hashear a biblioteca inteira à toa são minutos de I/O.
    content_hash BLOB,

    -- Metadata derivada das tags
    title           TEXT,
    album_id        INTEGER REFERENCES album(id),
    artist_id       INTEGER REFERENCES artist(id),
    album_artist_id INTEGER REFERENCES artist(id),
    disc_no         INTEGER,
    track_no        INTEGER,
    year            INTEGER,
    genre           TEXT,
    art_id          INTEGER REFERENCES cover_art(id),

    -- Propriedades do stream. `sample_rate` não é enfeite: o engine casa a taxa
    -- do device com a do arquivo pra não reamostrar, então ele é lido ANTES de
    -- abrir o device.
    duration_ms  INTEGER,
    sample_rate  INTEGER,
    channels     INTEGER,
    bit_depth    INTEGER,
    codec        TEXT,
    bitrate_kbps INTEGER,

    scanned_at INTEGER NOT NULL,

    UNIQUE (root_id, rel_path)
) STRICT;

-- Ordem natural de álbum: cobre "faixas do álbum X ordenadas" sem sort em memória.
CREATE INDEX track_by_album  ON track (album_id, disc_no, track_no);
CREATE INDEX track_by_artist ON track (artist_id);
-- Parcial: só as faixas já hasheadas interessam pro sync, e o índice fica
-- pequeno enquanto a maioria estiver NULL.
CREATE INDEX track_by_hash   ON track (content_hash) WHERE content_hash IS NOT NULL;
CREATE INDEX album_by_artist ON album (album_artist_id);

-- Busca. `remove_diacritics 2` faz "jose" achar "José" sem coluna normalizada
-- extra nem trabalho no lado do Rust.
-- rowid = track.id (join direto de volta, sem coluna de ligação).
-- Guarda cópia do texto (~4 MB em 50k faixas) em vez de usar contentless: em
-- troca, UPDATE e DELETE funcionam sem gerenciar rowid na mão.
CREATE VIRTUAL TABLE track_fts USING fts5 (
    title,
    artist,
    album,
    album_artist,
    tokenize = "unicode61 remove_diacritics 2"
);

-- -----------------------------------------------------------------------------
-- [B] Camada do usuário
-- -----------------------------------------------------------------------------

-- Um device = uma chave pública estática Noise. `id` é essa chave (32 bytes),
-- então parear e identificar são a mesma coisa.
CREATE TABLE device (
    id           BLOB PRIMARY KEY,
    name         TEXT    NOT NULL,
    is_self      INTEGER NOT NULL DEFAULT 0,
    paired_at    INTEGER,
    last_sync_at INTEGER
) STRICT;

CREATE TABLE playlist (
    id         BLOB PRIMARY KEY,        -- UUIDv7: ordenável no tempo, gerado offline
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,        -- LWW no merge
    -- Tombstone. Delete precisa sobreviver ao sync: sem isso, o outro device
    -- reintroduz a playlist apagada no próximo encontro.
    deleted    INTEGER NOT NULL DEFAULT 0,
    origin     BLOB    NOT NULL REFERENCES device(id)  -- desempate de updated_at igual
) STRICT;

CREATE TABLE playlist_item (
    playlist_id BLOB NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
    -- Índice fracionário ("a0", "a0V", "a1"). Inserir no meio escreve UMA
    -- linha em vez de renumerar a playlist inteira, e dois devices reordenando
    -- ao mesmo tempo fazem merge sem colidir.
    position    TEXT NOT NULL,
    track_key   BLOB NOT NULL,          -- content_hash, não track.id
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
) STRICT;

CREATE INDEX playlist_item_by_track ON playlist_item (track_key);

-- Contador por device (G-Counter). LWW aqui estaria ERRADO: se o PC tocou 3x e
-- o celular 2x, LWW guarda 3 e perde 2. O total é SUM(count), e o merge é
-- MAX(count) por (faixa, device) — converge sem perder play nenhum.
CREATE TABLE play_count (
    track_key BLOB    NOT NULL,
    device_id BLOB    NOT NULL REFERENCES device(id),
    count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (track_key, device_id)
) STRICT;

-- Campos escalares, cada um com seu próprio timestamp: dá pra mudar o rating
-- num device e a posição de retomada no outro sem um sobrescrever o outro.
CREATE TABLE track_state (
    track_key            BLOB PRIMARY KEY,
    last_played_at       INTEGER,
    rating               INTEGER,
    rating_updated_at    INTEGER NOT NULL DEFAULT 0,
    resume_pos_ms        INTEGER,
    resume_updated_at    INTEGER NOT NULL DEFAULT 0
) STRICT;

-- Chave/valor pra estado do próprio app (device_id, última pasta, volume).
-- Deliberadamente NÃO é tabela de configuração do usuário.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
