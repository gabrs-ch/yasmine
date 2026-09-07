//! Cache de capas, deduplicado por conteúdo.
//!
//! A mesma arte se repete em toda faixa de um álbum. Numa biblioteca de 50k
//! faixas isso são ~15 GB de bytes idênticos se tratados um a um, contra ~5k
//! imagens de verdade. Então tudo aqui gira em torno de uma coisa: **o blob é
//! identificado pelo BLAKE3 e só é decodificado na primeira vez que aparece.**
//!
//! O original nunca entra no banco nem fica na RAM. O que sobra em disco são
//! duas miniaturas prontas por capa:
//!
//! ```text
//! <cache>/art/<hex[0..2]>/<hex>_96.jpg    linha da lista
//! <cache>/art/<hex[0..2]>/<hex>_512.jpg   faixa tocando
//! ```
//!
//! Os dois primeiros hex viram diretório porque 5 000 arquivos numa pasta só
//! deixam o `readdir` lento em alguns sistemas de arquivos.
//!
//! **JPEG, não WebP.** O WebP do crate `image` só codifica sem perda, e uma
//! capa 512px sem perda fica maior que o JPEG de qualidade 85 e mais cara pra
//! decodificar na hora de desenhar a lista.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;

/// Lado maior de cada miniatura: uma pra linha da lista, uma pro player.
///
/// São *tetos*, não tamanhos fixos. Capa menor que o teto é reencodada no
/// tamanho original — ampliar não acrescenta detalhe nenhum e só gasta disco e
/// tempo de decodificação na hora de desenhar.
pub const THUMB_SIZES: [u32; 2] = [96, 512];

const JPEG_QUALITY: u8 = 85;

/// Entrada para semear o cache: hash da capa e dimensões do original.
pub type KnownArt = ([u8; 32], (u32, u32));

/// Referência a uma capa já materializada no cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtRef {
    pub hash: [u8; 32],
    pub width: u32,
    pub height: u32,
}

impl ArtRef {
    /// Caminho da miniatura de lado `size`.
    #[must_use]
    pub fn thumb_path(cache_dir: &Path, hash: &[u8; 32], size: u32) -> PathBuf {
        let hex = hex32(hash);
        cache_dir
            .join("art")
            .join(&hex[..2])
            .join(format!("{hex}_{size}.jpg"))
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Guarda quais capas já foram materializadas, para não decodificar duas vezes
/// a mesma imagem.
#[derive(Debug)]
pub struct ArtCache {
    dir: PathBuf,
    /// hash → dimensões do original. É a memória do dedupe: a segunda faixa
    /// do álbum encontra o hash aqui e nem chega a decodificar.
    known: Mutex<HashMap<[u8; 32], (u32, u32)>>,
}

impl ArtCache {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            known: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Semeia o cache com o que o índice já conhece, para um rescan não
    /// redecodificar capa nenhuma.
    pub fn preload(&self, entries: impl IntoIterator<Item = KnownArt>) {
        if let Ok(mut known) = self.known.lock() {
            known.extend(entries);
        }
    }

    /// Registra um blob de capa e devolve a referência.
    ///
    /// Na primeira vez decodifica e grava as miniaturas; nas seguintes só
    /// consulta o hash. Devolve `None` se a imagem não for decodificável —
    /// capa quebrada não pode impedir a faixa de entrar na biblioteca.
    pub fn store(&self, blob: &[u8]) -> Option<ArtRef> {
        let hash: [u8; 32] = blake3::hash(blob).into();

        if let Ok(known) = self.known.lock()
            && let Some(&(width, height)) = known.get(&hash)
        {
            return Some(ArtRef {
                hash,
                width,
                height,
            });
        }

        // Decodificar é o caro; acontece uma vez por capa distinta.
        let image = image::load_from_memory(blob).ok()?;
        let (width, height) = (image.width(), image.height());

        let longest = width.max(height);
        for size in THUMB_SIZES {
            // Teto, nunca ampliação.
            let target = size.min(longest).max(1);
            // `thumbnail` é amostragem por caixa: para reduções grandes é
            // muito mais rápido que Lanczos e a diferença não aparece em 96px.
            let thumb = image.thumbnail(target, target).to_rgb8();
            let path = ArtRef::thumb_path(&self.dir, &hash, size);
            if let Err(err) = write_jpeg(&path, &thumb) {
                // Cache é reconstruível: falhar aqui não invalida a faixa.
                eprintln!("capa {}: {err}", path.display());
            }
        }

        if let Ok(mut known) = self.known.lock() {
            known.insert(hash, (width, height));
        }

        Some(ArtRef {
            hash,
            width,
            height,
        })
    }

    /// Quantas capas distintas passaram por aqui.
    #[must_use]
    pub fn len(&self) -> usize {
        self.known.lock().map_or(0, |k| k.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Grava via arquivo temporário + rename: se o processo morrer no meio, o
/// cache fica sem a miniatura em vez de com meia miniatura.
fn write_jpeg(path: &Path, image: &image::RgbImage) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("jpg.tmp");
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(std::io::Error::other)?;

    fs::write(&tmp, &buf)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG 4x4 vermelho, gerado uma vez e colado aqui para o teste não
    /// depender de arquivo externo.
    fn png_valido() -> Vec<u8> {
        let mut img = image::RgbImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = image::Rgb([200, 30, 30]);
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("codificar png de teste");
        out
    }

    fn tmpdir(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("player-art-test-{nome}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn materializa_as_duas_miniaturas() {
        let dir = tmpdir("thumbs");
        let cache = ArtCache::new(dir.clone());
        let art = cache.store(&png_valido()).expect("capa válida");

        for size in THUMB_SIZES {
            let path = ArtRef::thumb_path(&dir, &art.hash, size);
            assert!(path.is_file(), "faltou {}", path.display());
        }
    }

    #[test]
    fn blob_repetido_nao_vira_capa_nova() {
        let cache = ArtCache::new(tmpdir("dedupe"));
        let blob = png_valido();

        let a = cache.store(&blob).expect("primeira");
        let b = cache.store(&blob).expect("segunda");

        assert_eq!(a, b);
        assert_eq!(cache.len(), 1, "o mesmo blob virou duas capas");
    }

    /// Capa pequena não pode virar um JPEG de 512px cheio de pixel inventado:
    /// isso enche o cache de disco e deixa o desenho da lista mais lento sem
    /// acrescentar detalhe nenhum.
    #[test]
    fn capa_menor_que_o_teto_nao_e_ampliada() {
        let dir = tmpdir("sem-upscale");
        let cache = ArtCache::new(dir.clone());
        let art = cache.store(&png_valido()).expect("capa válida");

        for size in THUMB_SIZES {
            let path = ArtRef::thumb_path(&dir, &art.hash, size);
            let thumb = image::open(&path).expect("abrir miniatura");
            assert!(
                thumb.width() <= 4 && thumb.height() <= 4,
                "miniatura {size} ficou {}x{} a partir de um original 4x4",
                thumb.width(),
                thumb.height()
            );
        }
    }

    #[test]
    fn imagem_quebrada_nao_derruba_o_scan() {
        let cache = ArtCache::new(tmpdir("quebrada"));
        assert!(cache.store(b"isto nao e uma imagem").is_none());
    }

    #[test]
    fn preload_evita_decodificar_de_novo() {
        let cache = ArtCache::new(tmpdir("preload"));
        let blob = png_valido();
        let hash: [u8; 32] = blake3::hash(&blob).into();

        cache.preload([(hash, (4, 4))]);
        let art = cache.store(&blob).expect("já conhecida");

        assert_eq!(art.width, 4);
        // Nada foi escrito: o preload disse que já estava materializada.
        assert!(!ArtRef::thumb_path(cache.dir(), &hash, 96).exists());
    }
}
