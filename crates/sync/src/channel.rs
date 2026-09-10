//! O canal cifrado: handshake Noise + transporte enquadrado de [`Msg`].
//!
//! Padrão **`Noise_IK_25519_ChaChaPoly_BLAKE2s`**. IK porque o iniciador (o
//! celular) já conhece a chave estática do respondedor (veio no QR), e manda a
//! própria estática cifrada na primeira mensagem — o respondedor descobre quem
//! é o par ao processar `msg1` e decide ali se aceita. Um pattern, os dois
//! casos: primeiro pareamento e reconexão.
//!
//! # Enquadramento
//!
//! Uma mensagem de transporte Noise carrega no máximo 65535 bytes (tag de 16
//! inclusa). Um `Msg::Blob` é maior que isso, então cada `Msg` lógico é
//! serializado, prefixado com `u32` de tamanho, e fatiado em records de
//! transporte; cada record vai ao socket com um prefixo `u16` do seu tamanho
//! cifrado. Quem recebe decifra records, acumula, e corta o `Msg` quando o
//! prefixo `u32` estiver completo.

use std::io::{Read, Write};

use snow::{Builder, TransportState};

use crate::identity::Identity;
use crate::protocol::{MAX_FRAME, Msg};
use crate::{Error, Result};

const PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
const NOISE_MSG_MAX: usize = 65535;
const NOISE_TAG_LEN: usize = 16;
const NOISE_PLAIN_MAX: usize = NOISE_MSG_MAX - NOISE_TAG_LEN;

pub struct Channel<S> {
    stream: S,
    noise: TransportState,
    /// Plaintext já decifrado mas ainda não cortado num `Msg` completo.
    rx: Vec<u8>,
}

impl<S: Read + Write> Channel<S> {
    /// Lado do celular: conhece a estática do host (do QR).
    pub fn initiator(mut stream: S, identity: &Identity, remote_static: &[u8; 32]) -> Result<Self> {
        let params = PARAMS
            .parse()
            .map_err(|_| Error::Noise("params Noise inválidos".into()))?;
        let mut hs = Builder::new(params)
            .local_private_key(identity.secret())
            .remote_public_key(remote_static)
            .build_initiator()?;

        let mut buf = vec![0u8; NOISE_MSG_MAX];
        let n = hs.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..n])?;

        let msg2 = read_raw(&mut stream)?;
        hs.read_message(&msg2, &mut buf)?;

        Ok(Self {
            stream,
            noise: hs.into_transport_mode()?,
            rx: Vec::new(),
        })
    }

    /// Lado do host: aprende a estática do par ao ler `msg1`. Devolve essa
    /// chave para quem chamou decidir se pareia.
    pub fn responder(mut stream: S, identity: &Identity) -> Result<(Self, [u8; 32])> {
        let params = PARAMS
            .parse()
            .map_err(|_| Error::Noise("params Noise inválidos".into()))?;
        let mut hs = Builder::new(params)
            .local_private_key(identity.secret())
            .build_responder()?;

        let mut buf = vec![0u8; NOISE_MSG_MAX];
        let msg1 = read_raw(&mut stream)?;
        hs.read_message(&msg1, &mut buf)?;

        let peer: [u8; 32] = hs
            .get_remote_static()
            .ok_or_else(|| Error::Noise("o par não mandou chave estática".into()))?
            .try_into()
            .map_err(|_| Error::Noise("chave estática do par com tamanho errado".into()))?;

        let n = hs.write_message(&[], &mut buf)?;
        write_raw(&mut stream, &buf[..n])?;

        Ok((
            Self {
                stream,
                noise: hs.into_transport_mode()?,
                rx: Vec::new(),
            },
            peer,
        ))
    }

    pub fn send(&mut self, msg: &Msg) -> Result<()> {
        let plain = postcard::to_stdvec(msg)?;
        let mut framed = Vec::with_capacity(plain.len() + 4);
        framed.extend_from_slice(
            &u32::try_from(plain.len())
                .map_err(|_| Error::Protocol("Msg grande demais".into()))?
                .to_be_bytes(),
        );
        framed.extend_from_slice(&plain);

        let mut out = Vec::with_capacity(framed.len() + 64);
        let mut ct = vec![0u8; NOISE_MSG_MAX];
        for piece in framed.chunks(NOISE_PLAIN_MAX) {
            let n = self.noise.write_message(piece, &mut ct)?;
            out.extend_from_slice(
                &u16::try_from(n)
                    .map_err(|_| Error::Protocol("record Noise > 64KiB".into()))?
                    .to_be_bytes(),
            );
            out.extend_from_slice(&ct[..n]);
        }
        self.stream.write_all(&out)?;
        self.stream.flush()?;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<Msg> {
        loop {
            if let Some(msg) = self.take_frame()? {
                return Ok(msg);
            }
            let record = read_raw(&mut self.stream)?;
            let mut plain = vec![0u8; record.len()];
            let n = self.noise.read_message(&record, &mut plain)?;
            plain.truncate(n);
            self.rx.extend_from_slice(&plain);
        }
    }

    fn take_frame(&mut self) -> Result<Option<Msg>> {
        if self.rx.len() < 4 {
            return Ok(None);
        }
        let need = u32::from_be_bytes([self.rx[0], self.rx[1], self.rx[2], self.rx[3]]) as usize;
        if need > MAX_FRAME {
            return Err(Error::Protocol("frame anunciado grande demais".into()));
        }
        if self.rx.len() < 4 + need {
            return Ok(None);
        }
        let frame: Vec<u8> = self.rx.drain(..4 + need).collect();
        Ok(Some(postcard::from_bytes(&frame[4..])?))
    }
}

fn read_raw<R: Read>(r: &mut R) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len)?;
    let mut buf = vec![0u8; u16::from_be_bytes(len) as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn write_raw<W: Write>(w: &mut W, bytes: &[u8]) -> Result<()> {
    let n = u16::try_from(bytes.len()).map_err(|_| Error::Protocol("record > 64KiB".into()))?;
    w.write_all(&n.to_be_bytes())?;
    w.write_all(bytes)?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use crate::protocol::CHUNK;

    /// Um par cliente/servidor sobre loopback, os dois já com identidade.
    fn par() -> (Channel<TcpStream>, Channel<TcpStream>) {
        let host = Identity::ephemeral();
        let phone = Identity::ephemeral();
        let host_pub = host.public();

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let h = thread::spawn(move || {
            let (sock, _) = listener.accept().expect("accept");
            let (ch, peer) = Channel::responder(sock, &host).expect("responder");
            (ch, peer)
        });

        let sock = TcpStream::connect(addr).expect("connect");
        let client = Channel::initiator(sock, &phone, &host_pub).expect("initiator");
        let (server, peer) = h.join().expect("join");
        assert_eq!(
            peer,
            phone.public(),
            "host não reconheceu a estática do celular"
        );
        (client, server)
    }

    #[test]
    fn troca_mensagem_pequena() {
        let (mut c, mut s) = par();
        c.send(&Msg::Hello {
            proto: 1,
            device_name: "celular".into(),
        })
        .expect("enviar");
        match s.recv().expect("receber") {
            Msg::Hello { proto, device_name } => {
                assert_eq!(proto, 1);
                assert_eq!(device_name, "celular");
            }
            outro => panic!("veio {outro:?}"),
        }
    }

    #[test]
    fn blob_maior_que_um_record_noise_e_remontado() {
        let (mut c, mut s) = par();
        let data = vec![0xABu8; CHUNK]; // 128 KiB > 65519, força fragmentação
        let hash = [1u8; 32];
        c.send(&Msg::Blob {
            hash,
            offset: 0,
            data: data.clone(),
            last: true,
        })
        .expect("enviar blob");
        match s.recv().expect("receber blob") {
            Msg::Blob {
                data: got, last, ..
            } => {
                assert!(last);
                assert_eq!(got, data);
            }
            outro => panic!("veio {outro:?}"),
        }
    }

    #[test]
    fn varias_mensagens_em_sequencia() {
        let (mut c, mut s) = par();
        for i in 0..50u16 {
            c.send(&Msg::NeedBlob {
                hash: [i as u8; 32],
                from: u64::from(i),
            })
            .expect("enviar");
        }
        for i in 0..50u16 {
            match s.recv().expect("receber") {
                Msg::NeedBlob { from, .. } => assert_eq!(from, u64::from(i)),
                outro => panic!("veio {outro:?}"),
            }
        }
    }
}
