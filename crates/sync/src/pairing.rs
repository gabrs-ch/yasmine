//! O payload do QR code.
//!
//! O QR carrega a **chave pública estática Noise do host** (que é o
//! `DeviceId`) e, opcionalmente, `host:porta` para dispensar o mDNS no
//! primeiro contato — útil em rede que bloqueia multicast. A chave vai sempre;
//! o endereço é dica, não obrigação.
//!
//! Formato: `yasmine://pair?k=<base64url>&h=<host>&p=<porta>&n=<nome>`

use data_encoding::BASE64URL_NOPAD;
use player_core::DeviceId;

use crate::{Error, Result};

const SCHEME: &str = "yasmine://pair?";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairPayload {
    /// Chave pública estática Noise do host = `DeviceId`.
    pub key: [u8; 32],
    /// IP ou hostname do host, quando o QR quis dispensar o mDNS.
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Nome legível, só para a UI ("Baixar biblioteca de <nome>?").
    pub name: String,
}

impl PairPayload {
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        DeviceId(self.key)
    }

    /// Endereço explícito do QR, se veio um.
    #[must_use]
    pub fn addr(&self) -> Option<std::net::SocketAddr> {
        let host = self.host.as_deref()?;
        let port = self.port?;
        // Só resolve forma literal de IP aqui; hostname fica pro mDNS/DNS.
        format!("{host}:{port}").parse().ok()
    }

    #[must_use]
    pub fn to_url(&self) -> String {
        let mut q = form_urlencoded::Serializer::new(String::new());
        q.append_pair("k", &BASE64URL_NOPAD.encode(&self.key));
        if let Some(h) = &self.host {
            q.append_pair("h", h);
        }
        if let Some(p) = self.port {
            q.append_pair("p", &p.to_string());
        }
        if !self.name.is_empty() {
            q.append_pair("n", &self.name);
        }
        format!("{SCHEME}{}", q.finish())
    }

    pub fn from_url(url: &str) -> Result<Self> {
        let query = url
            .strip_prefix(SCHEME)
            .or_else(|| url.strip_prefix("yasmine://pair/?"))
            .ok_or(Error::BadPairUrl("não começa com yasmine://pair?"))?;

        let mut key = None;
        let mut host = None;
        let mut port = None;
        let mut name = String::new();
        for (k, v) in form_urlencoded::parse(query.as_bytes()) {
            match k.as_ref() {
                "k" => {
                    let raw = BASE64URL_NOPAD
                        .decode(v.as_bytes())
                        .map_err(|_| Error::BadPairUrl("k não é base64url"))?;
                    key = Some(
                        <[u8; 32]>::try_from(raw.as_slice())
                            .map_err(|_| Error::BadPairUrl("k não tem 32 bytes"))?,
                    );
                }
                "h" => host = Some(v.into_owned()),
                "p" => {
                    port = Some(
                        v.parse::<u16>()
                            .map_err(|_| Error::BadPairUrl("p não é uma porta"))?,
                    );
                }
                "n" => name = v.into_owned(),
                _ => {}
            }
        }

        Ok(Self {
            key: key.ok_or(Error::BadPairUrl("falta k (a chave)"))?,
            host,
            port,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_e_volta_com_endereco() {
        let p = PairPayload {
            key: [7u8; 32],
            host: Some("192.168.0.42".into()),
            port: Some(45777),
            name: "PC do Gabriel".into(),
        };
        let url = p.to_url();
        assert_eq!(PairPayload::from_url(&url).expect("parse"), p);
        assert_eq!(
            p.addr(),
            Some("192.168.0.42:45777".parse().expect("addr literal"))
        );
    }

    #[test]
    fn ida_e_volta_so_com_a_chave() {
        let p = PairPayload {
            key: [1u8; 32],
            host: None,
            port: None,
            name: String::new(),
        };
        let back = PairPayload::from_url(&p.to_url()).expect("parse");
        assert_eq!(back, p);
        assert_eq!(back.addr(), None);
    }

    #[test]
    fn recusa_sem_chave() {
        assert!(matches!(
            PairPayload::from_url("yasmine://pair?n=PC"),
            Err(Error::BadPairUrl(_))
        ));
    }

    #[test]
    fn recusa_esquema_errado() {
        assert!(PairPayload::from_url("https://exemplo/pair?k=AAAA").is_err());
    }

    #[test]
    fn nome_com_espaco_e_acento_sobrevive() {
        let p = PairPayload {
            key: [9u8; 32],
            host: None,
            port: None,
            name: "Notebook da Yasmine ♪".into(),
        };
        assert_eq!(
            PairPayload::from_url(&p.to_url()).expect("parse").name,
            "Notebook da Yasmine ♪"
        );
    }
}
