//! A identidade estática deste device: um par de chaves X25519.
//!
//! A chave **pública** é o [`DeviceId`] — parear e identificar são a mesma
//! operação (o QR carrega essa chave, o handshake Noise a autentica). A chave
//! **secreta** mora em `meta['sync_sk']` no próprio índice: mesmo nível de
//! confiança do resto do arquivo, que já guarda a biblioteca inteira do
//! usuário.
//!
//! `player_core::playlist::self_device` cria, na primeira playlist, um
//! `device(is_self=1)` com id **aleatório provisório**. Quando a identidade de
//! verdade nasce aqui, esse id é reescrito para a chave pública — junto com
//! tudo que apontava para ele (`playlist.origin`, `play_count.device_id`).

use data_encoding::BASE64URL_NOPAD;
use player_core::db::now_ms;
use player_core::{Db, DeviceId};
use rusqlite::OptionalExtension;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::{Error, Result};

#[derive(Clone)]
pub struct Identity {
    secret: [u8; 32],
    public: [u8; 32],
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nunca imprimir a chave secreta, nem em log de debug.
        f.debug_struct("Identity")
            .field("public", &DeviceId(self.public))
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// Carrega a identidade do índice, criando na primeira vez.
    ///
    /// `device_name` só é usado na criação (nome legível do `device` self);
    /// chamadas seguintes ignoram.
    pub fn load_or_create(db: &mut Db, device_name: &str) -> Result<Self> {
        if let Some(stored) = read_meta(db, "sync_sk")? {
            let secret =
                decode_key(&stored).ok_or(Error::Protocol("meta['sync_sk'] corrompido".into()))?;
            let public = PublicKey::from(&StaticSecret::from(secret)).to_bytes();
            ensure_self_row(db, &public, device_name)?;
            return Ok(Self { secret, public });
        }

        let mut secret = [0u8; 32];
        getrandom::getrandom(&mut secret).map_err(|e| Error::Protocol(e.to_string()))?;
        let public = PublicKey::from(&StaticSecret::from(secret)).to_bytes();

        let tx = db.conn_mut().transaction()?;
        // Repontar filhos antes de mexer no `device` sem tropeçar na FK.
        tx.execute_batch("PRAGMA defer_foreign_keys = TRUE")?;

        let provisional: Option<Vec<u8>> = tx
            .query_row("SELECT id FROM device WHERE is_self = 1", [], |r| r.get(0))
            .optional()?;
        match provisional {
            Some(old) if old.as_slice() != public => {
                tx.execute(
                    "UPDATE playlist SET origin = ?1 WHERE origin = ?2",
                    rusqlite::params![public.as_slice(), old.as_slice()],
                )?;
                tx.execute(
                    "UPDATE play_count SET device_id = ?1 WHERE device_id = ?2",
                    rusqlite::params![public.as_slice(), old.as_slice()],
                )?;
                tx.execute(
                    "UPDATE device SET id = ?1 WHERE id = ?2",
                    rusqlite::params![public.as_slice(), old.as_slice()],
                )?;
            }
            Some(_) => {}
            None => {
                tx.execute(
                    "INSERT INTO device (id, name, is_self, paired_at) VALUES (?1, ?2, 1, ?3)",
                    rusqlite::params![public.as_slice(), device_name, now_ms()],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('sync_sk', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [encode_key(&secret)],
        )?;
        tx.commit()?;

        Ok(Self { secret, public })
    }

    /// Identidade efêmera, sem tocar no índice — para testes e para o
    /// primeiro contato antes de haver `Db`.
    #[must_use]
    pub fn ephemeral() -> Self {
        let mut secret = [0u8; 32];
        getrandom::getrandom(&mut secret).expect("getrandom");
        let public = PublicKey::from(&StaticSecret::from(secret)).to_bytes();
        Self { secret, public }
    }

    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        DeviceId(self.public)
    }

    #[must_use]
    pub(crate) fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    #[must_use]
    pub fn public(&self) -> [u8; 32] {
        self.public
    }
}

fn read_meta(db: &Db, key: &str) -> Result<Option<String>> {
    Ok(db
        .conn()
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

fn ensure_self_row(db: &Db, public: &[u8; 32], device_name: &str) -> Result<()> {
    let existing: Option<Vec<u8>> = db
        .conn()
        .query_row("SELECT id FROM device WHERE is_self = 1", [], |r| r.get(0))
        .optional()?;
    match existing {
        Some(id) if id.as_slice() == public => Ok(()),
        Some(old) => {
            let tx = db.conn().unchecked_transaction()?;
            tx.execute_batch("PRAGMA defer_foreign_keys = TRUE")?;
            tx.execute(
                "UPDATE playlist SET origin = ?1 WHERE origin = ?2",
                rusqlite::params![public.as_slice(), old.as_slice()],
            )?;
            tx.execute(
                "UPDATE play_count SET device_id = ?1 WHERE device_id = ?2",
                rusqlite::params![public.as_slice(), old.as_slice()],
            )?;
            tx.execute(
                "UPDATE device SET id = ?1 WHERE id = ?2",
                rusqlite::params![public.as_slice(), old.as_slice()],
            )?;
            tx.commit()?;
            Ok(())
        }
        None => {
            db.conn().execute(
                "INSERT INTO device (id, name, is_self, paired_at) VALUES (?1, ?2, 1, ?3)",
                rusqlite::params![public.as_slice(), device_name, now_ms()],
            )?;
            Ok(())
        }
    }
}

fn encode_key(bytes: &[u8; 32]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

fn decode_key(s: &str) -> Option<[u8; 32]> {
    let raw = BASE64URL_NOPAD.decode(s.as_bytes()).ok()?;
    <[u8; 32]>::try_from(raw.as_slice()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().expect("índice em memória")
    }

    #[test]
    fn cria_uma_vez_e_reusa() {
        let mut db = db();
        let a = Identity::load_or_create(&mut db, "PC").expect("criar");
        let b = Identity::load_or_create(&mut db, "PC").expect("reusar");
        assert_eq!(a.public(), b.public());
        assert_eq!(a.secret(), b.secret());
    }

    #[test]
    fn a_chave_publica_vira_o_device_self() {
        let mut db = db();
        let id = Identity::load_or_create(&mut db, "PC do Gabriel").expect("criar");

        let (row_id, name): (Vec<u8>, String) = db
            .conn()
            .query_row("SELECT id, name FROM device WHERE is_self = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("device self");
        assert_eq!(row_id.as_slice(), id.public());
        assert_eq!(name, "PC do Gabriel");
    }

    #[test]
    fn adota_o_device_provisorio_e_o_que_apontava_pra_ele() {
        let mut db = db();
        // Simula o estado pré-Fase-4: playlist criada com um self-id aleatório.
        let pl = player_core::playlist::create(&db, "Antiga").expect("playlist");
        let velho: Vec<u8> = db
            .conn()
            .query_row(
                "SELECT origin FROM playlist WHERE id = ?1",
                [pl.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .expect("origin");

        let id = Identity::load_or_create(&mut db, "PC").expect("criar identidade");

        let novo: Vec<u8> = db
            .conn()
            .query_row(
                "SELECT origin FROM playlist WHERE id = ?1",
                [pl.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .expect("origin");
        assert_ne!(novo, velho, "origin não foi repontado");
        assert_eq!(novo.as_slice(), id.public());

        let selves: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM device WHERE is_self = 1", [], |r| {
                r.get(0)
            })
            .expect("contar");
        assert_eq!(selves, 1, "sobrou device self duplicado");
    }
}
