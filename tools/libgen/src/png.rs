//! Codificador PNG mínimo, sem dependência.
//!
//! Usa blocos deflate "stored" (sem compressão): o objetivo é produzir uma
//! capa *válida* e de bytes previsíveis, não uma pequena. Cada álbum recebe
//! uma imagem diferente, e todas as faixas do álbum recebem a mesma — é
//! exatamente esse padrão que a deduplicação por BLAKE3 tem que explorar.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());

    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Envolve os bytes num stream zlib de blocos "stored".
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]); // CM=deflate, janela 32K, sem dicionário

    let mut chunks = raw.chunks(65535).peekable();
    while let Some(block) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        let len = u16::try_from(block.len()).unwrap_or(u16::MAX);
        out.push(last); // BFINAL + BTYPE=00 (stored)
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }

    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// Gera um PNG RGB `size`x`size` com um padrão derivado de `seed`.
#[must_use]
pub fn cover(size: u32, seed: u64) -> Vec<u8> {
    let (r0, g0, b0) = (
        (seed & 0xFF) as u8,
        ((seed >> 8) & 0xFF) as u8,
        ((seed >> 16) & 0xFF) as u8,
    );

    // Scanlines com filtro 0 (None) — nenhum decodificador precisa desfazer nada.
    let mut raw = Vec::with_capacity((size * (size * 3 + 1)) as usize);
    for y in 0..size {
        raw.push(0);
        for x in 0..size {
            let fade = ((x + y) * 255 / (2 * size.max(1))) as u8;
            raw.push(r0.wrapping_add(fade));
            raw.push(g0.wrapping_sub(fade));
            raw.push(b0 ^ fade);
        }
    }

    let mut png = Vec::with_capacity(raw.len() + 64);
    png.extend_from_slice(&SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bits, RGB, sem interlace
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);

    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gera_png_com_assinatura_e_iend() {
        let png = cover(16, 42);
        assert_eq!(&png[..8], &SIGNATURE);
        assert!(png.ends_with(&[0xAE, 0x42, 0x60, 0x82]), "CRC do IEND");
    }

    #[test]
    fn seeds_diferentes_geram_capas_diferentes() {
        assert_ne!(cover(16, 1), cover(16, 2));
    }

    #[test]
    fn mesma_seed_gera_bytes_identicos() {
        assert_eq!(cover(16, 7), cover(16, 7));
    }
}
