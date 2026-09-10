//! BLAKE3 de 32 bytes ⇄ hex. É a chave da capa no cache (`ArtRef::thumb_path`)
//! e o que vai na URL `art://…/<hex>/<size>` que a `<img>` do front pede.

use std::fmt::Write as _;

pub fn encode(hash: &[u8; 32]) -> String {
    hash.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn decode(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}
