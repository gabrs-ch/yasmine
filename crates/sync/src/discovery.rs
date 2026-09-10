//! Descoberta na LAN por mDNS.
//!
//! O host publica `_yasmine-sync._tcp.local.` com um TXT `id=<hex da chave
//! pública>` e `name=<nome>`. O celular navega o mesmo tipo e casa pelo `id`
//! que já tem do pareamento. IP digitado à mão é o plano B (rede sem
//! multicast) — nesse caso nada aqui é usado.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use data_encoding::HEXLOWER;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use player_core::DeviceId;

use crate::{Error, Result, SERVICE_TYPE};

/// Registro mDNS ativo. Cai (desregistra) quando isto é derrubado.
pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

/// Publica este device na LAN. `instance` é um nome curto e estável (usa-se o
/// começo do hex da chave), `name` é o rótulo legível que vai no TXT.
pub fn advertise(id: &DeviceId, port: u16, name: &str) -> Result<Advertisement> {
    let daemon = ServiceDaemon::new().map_err(Error::from)?;
    let hex = HEXLOWER.encode(id.as_bytes());
    let instance = &hex[..16];
    let host = format!("yasmine-{}.local.", &hex[..12]);

    let info = ServiceInfo::new(
        SERVICE_TYPE,
        instance,
        &host,
        "",
        port,
        &[("id", hex.as_str()), ("name", name)][..],
    )
    .map_err(Error::from)?
    .enable_addr_auto();

    let fullname = info.get_fullname().to_string();
    daemon.register(info).map_err(Error::from)?;
    Ok(Advertisement { daemon, fullname })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub id: DeviceId,
    pub name: String,
    pub addr: SocketAddr,
}

/// Navegador contínuo: `poll` drena o que resolveu desde a última vez.
pub struct Discovery {
    daemon: ServiceDaemon,
    rx: mdns_sd::Receiver<ServiceEvent>,
}

impl Discovery {
    pub fn start() -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(Error::from)?;
        let rx = daemon.browse(SERVICE_TYPE).map_err(Error::from)?;
        Ok(Self { daemon, rx })
    }

    /// Devices resolvidos desde a última chamada. Não bloqueia.
    pub fn poll(&self) -> Vec<Discovered> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            if let ServiceEvent::ServiceResolved(info) = event
                && let Some(d) = from_info(&info)
            {
                out.push(d);
            }
        }
        out
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
    }
}

/// Resolve um device específico de uma vez, com prazo. Usado quando o QR não
/// trouxe `host:porta` e a UI só quer conectar.
pub fn resolve_once(id: &DeviceId, timeout: Duration) -> Result<Option<SocketAddr>> {
    let disco = Discovery::start()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for d in disco.poll() {
            if d.id == *id {
                return Ok(Some(d.addr));
            }
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    Ok(None)
}

fn from_info(info: &ServiceInfo) -> Option<Discovered> {
    let props = info.get_properties();
    let hex = props.get_property_val_str("id")?;
    let raw = HEXLOWER.decode(hex.as_bytes()).ok()?;
    let key = <[u8; 32]>::try_from(raw.as_slice()).ok()?;
    let name = props.get_property_val_str("name").unwrap_or("").to_string();
    let ip = info.get_addresses().iter().next().copied()?;
    Some(Discovered {
        id: DeviceId(key),
        name,
        addr: SocketAddr::new(ip, info.get_port()),
    })
}
