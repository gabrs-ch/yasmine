//! Leitura e escrita do nivelador de volume no índice.
//!
//! A medição em si — decodificar a faixa inteira e calcular RMS/pico — mora
//! em `player_audio::loudness`: este crate não depende de codec de áudio, só
//! de dados. Aqui fica só o que é consulta: quais faixas ainda não foram
//! medidas, e gravar o resultado quando uma tarefa de fundo termina uma.

use std::path::PathBuf;

use crate::db::{Db, Error, Result};
use crate::model::TrackId;

/// Uma faixa ainda sem dado de nivelador, com o caminho pronto pra decodificar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: TrackId,
    pub path: PathBuf,
}

/// Até `limit` faixas sem nivelador calculado, em ordem de id.
///
/// Processar em lotes pequenos (não a biblioteca inteira de uma vez) é o que
/// deixa a tarefa de fundo interrompível: o app fecha, ou uma nova pasta é
/// escaneada, sem um lote gigante de decodificação preso no meio.
pub fn pending(db: &Db, limit: usize) -> Result<Vec<Pending>> {
    let mut stmt = db.conn().prepare(
        "SELECT t.id, r.path, t.rel_path FROM track t
         JOIN library_root r ON r.id = t.root_id
         WHERE t.loudness_gain_db IS NULL
         ORDER BY t.id
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |row| {
        Ok(Pending {
            id: TrackId(row.get(0)?),
            path: PathBuf::from(row.get::<_, String>(1)?).join(row.get::<_, String>(2)?),
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Error::from)
}

/// Grava o ganho e o pico medidos para uma faixa.
pub fn set(db: &Db, id: TrackId, gain_db: f32, peak: f32) -> Result<()> {
    db.conn().execute(
        "UPDATE track SET loudness_gain_db = ?2, loudness_peak = ?3 WHERE id = ?1",
        rusqlite::params![id.0, gain_db, peak],
    )?;
    Ok(())
}

/// Quantas faixas ainda faltam medir — para saber quando a tarefa de fundo
/// pode parar.
pub fn remaining(db: &Db) -> Result<u64> {
    let count: i64 = db.conn().query_row(
        "SELECT count(*) FROM track WHERE loudness_gain_db IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::art::ArtCache;
    use crate::scan::scan;
    use crate::testutil::{ambiente, escreve};

    fn biblioteca(nome: &str, faixas: usize) -> (Db, crate::testutil::Ambiente) {
        let env = ambiente(nome);
        for n in 0..faixas {
            escreve(
                &env.musica.join(format!("a/{n:02}.mp3")),
                &format!("Faixa {n:02}"),
                "Artista",
                "Álbum",
                None,
            );
        }
        let mut db = Db::open_in_memory().expect("abrir");
        let art = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &art).expect("escanear");
        (db, env)
    }

    #[test]
    fn tudo_pendente_logo_apos_o_scan() {
        let (db, _env) = biblioteca("loudness-pendente", 3);
        assert_eq!(pending(&db, 10).expect("consultar").len(), 3);
        assert_eq!(remaining(&db).expect("contar"), 3);
    }

    #[test]
    fn medir_uma_faixa_tira_ela_da_fila() {
        let (db, _env) = biblioteca("loudness-medida", 3);
        let alvo = pending(&db, 10).expect("consultar")[0].id;

        set(&db, alvo, -3.0, 0.8).expect("gravar");

        assert_eq!(remaining(&db).expect("contar"), 2);
        let restantes = pending(&db, 10).expect("consultar");
        assert!(restantes.iter().all(|p| p.id != alvo));
    }

    #[test]
    fn limit_e_respeitado() {
        let (db, _env) = biblioteca("loudness-lote", 5);
        assert_eq!(pending(&db, 2).expect("consultar").len(), 2);
    }

    /// Reeditar uma faixa some com o nivelador dela: o áudio pode ter
    /// mudado, o ganho antigo não vale mais.
    #[test]
    fn reeditar_a_faixa_volta_ela_pra_fila() {
        let (db, env) = biblioteca("loudness-reeditada", 1);
        let alvo = pending(&db, 10).expect("consultar")[0].id;
        set(&db, alvo, -3.0, 0.8).expect("gravar");
        assert_eq!(remaining(&db).expect("contar"), 0);

        escreve(
            &env.musica.join("a/00.mp3"),
            "Faixa Editada",
            "Artista",
            "Álbum",
            None,
        );
        let art = ArtCache::new(env.cache.clone());
        let mut db = db;
        scan(&mut db, &env.musica, &art).expect("reescanear");

        assert_eq!(remaining(&db).expect("contar"), 1);
    }
}
