//! Abertura, PRAGMAs e migração do índice.

use std::path::Path;

use rusqlite::Connection;

/// Versão do schema gravada em `PRAGMA user_version`.
pub const SCHEMA_VERSION: i32 = 5;

const SCHEMA_SQL: &str = include_str!("schema.sql");

/// PRAGMAs de conexão. Todos são decisões de performance, nenhum é opção do
/// usuário.
///
/// - `WAL`: leitura não bloqueia escrita. A UI consulta enquanto o scanner
///   grava, sem travar a lista.
/// - `synchronous = NORMAL`: sob WAL isso só abre mão de durabilidade se a
///   *máquina* cair no meio de um commit — e o pior caso é reescanear. Vale o
///   fsync a menos por transação num scan de 50k faixas.
/// - `mmap_size = 256 MB`: lê páginas direto do page cache do SO, sem copiar
///   pro buffer do SQLite.
/// - `cache_size = -4000`: 4 MB (o sinal negativo é KiB, não páginas). Medido
///   sobre 50 000 faixas, 16 MB e 1 MB dão o mesmo tempo de consulta — com o
///   `mmap` ligado, quem serve as páginas é o cache do SO, e o cache próprio
///   do SQLite vira memória parada.
const PRAGMAS: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous  = NORMAL;
    PRAGMA foreign_keys = ON;
    PRAGMA temp_store   = MEMORY;
    PRAGMA mmap_size    = 268435456;
    PRAGMA cache_size   = -4000;
    PRAGMA busy_timeout = 5000;
";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("erro de sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("erro de e/s: {0}")]
    Io(#[from] std::io::Error),

    /// Downgrade do app com índice novo. Melhor recusar do que corromper.
    #[error(
        "o índice foi criado por uma versão mais nova do player \
         (schema v{found}, esta versão entende até v{supported})"
    )]
    SchemaTooNew { found: i32, supported: i32 },
}

pub type Result<T> = std::result::Result<T, Error>;

/// O índice da biblioteca.
///
/// Uma única conexão. Vale o lembrete pro Android: o Rust é o **único**
/// escritor deste arquivo — o Kotlin consulta via FFI, nunca abre o SQLite
/// direto. Dois escritores no mesmo arquivo dá corrupção difícil de rastrear.
#[derive(Debug)]
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Abre (criando se preciso) o índice em `path` e aplica as migrações.
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_conn(Connection::open(path)?)
    }

    /// Índice efêmero — testes e benchmarks.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.execute_batch(PRAGMAS)?;
        // Habilita `rarray(?)`, que deixa buscar uma janela inteira de ids
        // numa consulta só em vez de uma por linha visível.
        rusqlite::vtab::array::load_module(&conn)?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if version > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        if version == SCHEMA_VERSION {
            return Ok(());
        }

        // Migrações são aplicadas em ordem, cada uma numa transação. Da v1 em
        // diante, cada passo novo entra aqui como um braço a mais — nunca
        // editando o schema.sql, que descreve só a criação do zero.
        let tx = self.conn.transaction()?;
        if version < 1 {
            tx.execute_batch(SCHEMA_SQL)?;
        }
        if version < 2 {
            // Nivelador de volume: ganho e pico calculados uma vez por
            // faixa (ver `player_audio::loudness`), preguiçoso como o
            // content_hash — nasce NULL, uma tarefa de fundo preenche depois
            // do scan sem atrasar a lista ficar pronta.
            tx.execute_batch(
                "ALTER TABLE track ADD COLUMN loudness_gain_db REAL;
                 ALTER TABLE track ADD COLUMN loudness_peak REAL;",
            )?;
        }
        if version < 3 {
            // Playlist vinculada a pasta (ver player_core::playlist_folder):
            // tabela nova, não coluna, então não cabe num ALTER TABLE.
            tx.execute_batch(
                "CREATE TABLE playlist_folder (
                    playlist_id BLOB    NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
                    root_id     INTEGER NOT NULL REFERENCES library_root(id) ON DELETE CASCADE,
                    rel_prefix  TEXT    NOT NULL,
                    PRIMARY KEY (playlist_id, root_id, rel_prefix)
                 ) STRICT;
                 CREATE INDEX playlist_folder_by_root ON playlist_folder (root_id, rel_prefix);",
            )?;
        }
        if version < 4 {
            // Capa da playlist escolhida pelo usuário: o BLAKE3 do blob (as
            // miniaturas ficam no mesmo cache em disco das capas de álbum).
            // NULL = sem capa própria, a UI cai na capa da primeira faixa.
            // Coluna na tabela `playlist`, então viaja no mesmo LWW do resto
            // da linha no sync.
            tx.execute_batch("ALTER TABLE playlist ADD COLUMN image_hash BLOB;")?;
        }
        if version < 5 {
            // Túmulo de item de playlist. Apagar um item passa a marcar
            // `deleted = 1` em vez de sumir com a linha: o sync (repo
            // companion) faz UNIÃO dos itens das duas pontas, então sem o
            // túmulo o item apagado num device é reintroduzido pelo outro.
            // `deleted_at` é o relógio do LWW que decide "apagado depois de
            // re-adicionado?". Mesma regra das v2–v4: coluna via migração, o
            // schema.sql descreve só a criação do zero.
            tx.execute_batch(
                "ALTER TABLE playlist_item ADD COLUMN deleted    INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE playlist_item ADD COLUMN deleted_at INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;

        Ok(())
    }

    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    #[must_use]
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// Agora, em milissegundos desde a época.
///
/// É o relógio de tudo que é gravado: `scanned_at`, `updated_at` das
/// playlists, timestamps de LWW no sync.
#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cria_schema_do_zero() {
        let db = Db::open_in_memory().expect("abrir índice em memória");
        let version: i32 = db
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("ler user_version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn migrar_de_novo_e_no_op() {
        let db = Db::open_in_memory().expect("abrir");
        let mut db = db;
        db.migrate().expect("segunda migração não deve falhar");
    }

    /// Guarda-chuva: o FTS5 precisa estar compilado no SQLite embutido, senão
    /// a busca some sem aviso.
    #[test]
    fn fts5_esta_disponivel_e_dobra_acento() {
        let db = Db::open_in_memory().expect("abrir");
        db.conn()
            .execute(
                "INSERT INTO track_fts (rowid, title, artist, album, album_artist)
                 VALUES (1, 'Eduardo e Mônica', 'Legião Urbana', 'Dois', 'Legião Urbana')",
                [],
            )
            .expect("inserir no fts");

        // Busca sem acento tem que achar o registro acentuado.
        let hits: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM track_fts WHERE track_fts MATCH 'monica'",
                [],
                |r| r.get(0),
            )
            .expect("consultar fts");
        assert_eq!(hits, 1, "remove_diacritics não está ativo");
    }

    #[test]
    fn recusa_indice_de_versao_futura() {
        let conn = Connection::open_in_memory().expect("abrir conexão");
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("marcar versão futura");

        let err = Db::from_conn(conn).expect_err("deveria recusar");
        assert!(matches!(err, Error::SchemaTooNew { .. }));
    }

    /// Índice "antigo": só o schema base, carimbado como v4 na mão. Reabrir
    /// tem que aplicar o passo v5 (túmulo de `playlist_item`) e nada mais.
    #[test]
    fn migra_v4_para_v5_adiciona_tumulo_de_item() {
        let conn = Connection::open_in_memory().expect("abrir conexão");
        conn.execute_batch(SCHEMA_SQL).expect("schema base");
        conn.pragma_update(None, "user_version", 4)
            .expect("marcar v4");

        let db = Db::from_conn(conn).expect("migrar para v5");

        let version: i32 = db
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("ler user_version");
        assert_eq!(version, SCHEMA_VERSION);

        let colunas: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM pragma_table_info('playlist_item')
                 WHERE name IN ('deleted', 'deleted_at')",
                [],
                |r| r.get(0),
            )
            .expect("consultar colunas");
        assert_eq!(colunas, 2, "o túmulo de playlist_item não foi criado");
    }
}
