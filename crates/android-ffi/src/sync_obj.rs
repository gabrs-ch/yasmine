//! `Syncer`: pareamento por QR, descoberta na LAN e o `pull` da biblioteca do
//! host. Envolve `yasmine-sync`, compartilhando o mesmo `Db` do [`YasmineLibrary`].

use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use data_encoding::HEXLOWER;
use yasmine_sync::{Discovery, Identity, PairPayload, Target};

use crate::types::{DiscoveredFfi, PairInfoFfi, PairedDeviceFfi, PullReportFfi, SyncListener};
use crate::{FfiError, Result, YasmineLibrary};

#[derive(uniffi::Object)]
pub struct Syncer {
    library: Arc<YasmineLibrary>,
    identity: Identity,
    cancel: Arc<AtomicBool>,
    discovery: Mutex<Option<Discovery>>,
}

#[uniffi::export]
impl Syncer {
    /// Carrega (ou cria no primeiro boot) a identidade estática deste device,
    /// guardada no próprio índice.
    #[uniffi::constructor]
    pub fn new(library: Arc<YasmineLibrary>, device_name: String) -> Result<Arc<Self>> {
        let identity = {
            let mut db = library.lock()?;
            Identity::load_or_create(&mut db, &device_name)?
        };
        Ok(Arc::new(Self {
            library,
            identity,
            cancel: Arc::new(AtomicBool::new(false)),
            discovery: Mutex::new(None),
        }))
    }

    /// A chave pública deste device em hex — o que o PC lê se o pareamento for
    /// bidirecional por QR.
    pub fn self_device_id(&self) -> String {
        HEXLOWER.encode(self.identity.device_id().as_bytes())
    }

    /// Interpreta um QR `yasmine://pair?...`.
    pub fn parse_pair(&self, url: String) -> Result<PairInfoFfi> {
        Ok(PairPayload::from_url(&url)?.into())
    }

    pub fn start_discovery(&self) -> Result<()> {
        let mut slot = self
            .discovery
            .lock()
            .map_err(|_| FfiError::msg("descoberta travada"))?;
        if slot.is_none() {
            *slot = Some(Discovery::start()?);
        }
        Ok(())
    }

    pub fn stop_discovery(&self) {
        if let Ok(mut slot) = self.discovery.lock() {
            *slot = None;
        }
    }

    /// Devices vistos na LAN desde a última chamada.
    pub fn discovered(&self) -> Vec<DiscoveredFfi> {
        let Ok(slot) = self.discovery.lock() else {
            return Vec::new();
        };
        slot.as_ref()
            .map(|d| d.poll().into_iter().map(Into::into).collect())
            .unwrap_or_default()
    }

    pub fn paired_devices(&self) -> Result<Vec<PairedDeviceFfi>> {
        let db = self.library.lock()?;
        let mut stmt = db.conn().prepare(
            "SELECT id, name, paired_at, last_sync_at FROM device
             WHERE is_self = 0 ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PairedDeviceFfi {
                device_id: HEXLOWER.encode(&r.get::<_, Vec<u8>>(0)?),
                name: r.get(1)?,
                paired_at: r.get(2)?,
                last_sync_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Esquece um device pareado. As playlists que ele originou passam a ser
    /// atribuídas a este device (a linha do outro não pode sumir com a FK
    /// apontando pra ela).
    pub fn unpair(&self, device_id_hex: String) -> Result<()> {
        let id = HEXLOWER
            .decode(device_id_hex.as_bytes())
            .map_err(FfiError::msg)?;
        let db = self.library.lock()?;
        let self_id: Vec<u8> =
            db.conn()
                .query_row("SELECT id FROM device WHERE is_self = 1", [], |r| r.get(0))?;
        let tx = db.conn().unchecked_transaction()?;
        tx.execute_batch("PRAGMA defer_foreign_keys = TRUE")?;
        tx.execute(
            "UPDATE playlist SET origin = ?1 WHERE origin = ?2",
            rusqlite::params![self_id, id],
        )?;
        tx.execute("DELETE FROM play_count WHERE device_id = ?1", [&id])?;
        tx.execute("DELETE FROM device WHERE id = ?1 AND is_self = 0", [&id])?;
        tx.commit()?;
        Ok(())
    }

    /// Interrompe um `pull` em andamento — o que já baixou fica, a próxima
    /// chamada retoma.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Baixa a biblioteca inteira do host descrito no QR. Bloqueante — o
    /// Kotlin chama de `Dispatchers.IO`.
    pub fn pull(
        &self,
        pair_url: String,
        dest_dir: String,
        listener: Box<dyn SyncListener>,
    ) -> Result<PullReportFfi> {
        let payload = PairPayload::from_url(&pair_url)?;
        self.pull_impl(payload_target(&payload)?, dest_dir, listener)
    }

    /// Igual, mas com endereço `ip:porta` digitado à mão (rede sem mDNS).
    pub fn pull_from_addr(
        &self,
        device_id_hex: String,
        addr: String,
        dest_dir: String,
        listener: Box<dyn SyncListener>,
    ) -> Result<PullReportFfi> {
        let raw = HEXLOWER
            .decode(device_id_hex.as_bytes())
            .map_err(FfiError::msg)?;
        let key: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| FfiError::msg("device_id precisa de 32 bytes"))?;
        let sa = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| FfiError::msg("endereço inválido"))?;
        self.pull_impl(
            Target::Addr {
                addr: sa,
                id: player_core::DeviceId(key),
            },
            dest_dir,
            listener,
        )
    }
}

impl Syncer {
    fn pull_impl(
        &self,
        target: Target,
        dest_dir: String,
        listener: Box<dyn SyncListener>,
    ) -> Result<PullReportFfi> {
        self.cancel.store(false, Ordering::Relaxed);
        let dest = PathBuf::from(dest_dir);
        let cache = self.library.cache_dir.clone();
        let mut db = self.library.lock()?;
        let report = yasmine_sync::pull(
            &mut db,
            &self.identity,
            target,
            &dest,
            &cache,
            &|p| listener.on_progress(p.into()),
            &self.cancel,
        )?;
        Ok(report.into())
    }
}

fn payload_target(p: &PairPayload) -> Result<Target> {
    if let Some(addr) = p.addr() {
        return Ok(Target::Addr {
            addr,
            id: p.device_id(),
        });
    }
    if let (Some(h), Some(port)) = (&p.host, p.port)
        && let Some(addr) = (h.as_str(), port).to_socket_addrs()?.next()
    {
        return Ok(Target::Addr {
            addr,
            id: p.device_id(),
        });
    }
    Ok(Target::Discovered(p.device_id()))
}
