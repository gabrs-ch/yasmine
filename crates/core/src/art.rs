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
//! capa 512px sem perda fica maior que o JPEG de qualidade 90 e mais cara pra
//! decodificar na hora de desenhar a lista.
//!
//! **Reamostragem Lanczos3, não amostragem por caixa.** A redução acontece
//! uma vez por capa distinta, dentro do worker paralelo do scan (que é I/O
//! bound de qualquer jeito), então o custo a mais do Lanczos não aparece no
//! relógio — mas a diferença aparece na tela: caixa deixa a capa mole e
//! serrilhada, Lanczos mantém o contorno.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;

use crate::db::{Db, Result};

/// Lado maior de cada miniatura: uma pra linha da lista, uma pro player.
///
/// São *tetos*, não tamanhos fixos. Capa menor que o teto é reencodada no
/// tamanho original — ampliar não acrescenta detalhe nenhum e só gasta disco e
/// tempo de decodificação na hora de desenhar.
pub const THUMB_SIZES: [u32; 2] = [96, 512];

const JPEG_QUALITY: u8 = 90;

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
            let thumb = if target >= longest {
                // Capa já cabe: nada a reduzir, só reencodar.
                image.to_rgb8()
            } else {
                // Escala proporcional pelo maior lado (capa não é sempre
                // quadrada), com Lanczos3 — o filtro que preserva o
                // contorno na redução.
                let scale = f64::from(target) / f64::from(longest);
                let w = ((f64::from(width) * scale).round() as u32).max(1);
                let h = ((f64::from(height) * scale).round() as u32).max(1);
                image
                    .resize_exact(w, h, image::imageops::FilterType::Lanczos3)
                    .to_rgb8()
            };
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

/// Contador de nomes temporários.
///
/// Dois workers podem hashear o mesmo blob antes de qualquer um registrar o
/// resultado, e aí os dois materializam a mesma capa. Com um nome temporário
/// fixo, o primeiro `rename` levava o arquivo embora e o segundo falhava com
/// ENOENT. Um sufixo único por gravação resolve: os dois renomeiam para o
/// mesmo destino final, e `rename` é atômico.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Grava via arquivo temporário + rename: se o processo morrer no meio, o
/// cache fica sem a miniatura em vez de com meia miniatura.
fn write_jpeg(path: &Path, image: &image::RgbImage) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("jpg.{seq}.tmp"));
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

/// Aponta a capa do álbum de `track` para o conteúdo de `blob` — o caso de
/// "escolher uma imagem pra essa música", que na prática é sempre uma
/// imagem de álbum.
///
/// `cache.store` só materializa as miniaturas em disco; quem grava a linha
/// em `cover_art` sempre foi o escritor do scan (`scan.rs`), então essa
/// função repete o mesmo `INSERT ... ON CONFLICT DO NOTHING` antes de
/// apontar o álbum pra ela — sem isso, uma capa escolhida pelo usuário
/// nunca teria linha na tabela.
///
/// Atualiza o álbum, não a faixa: é o álbum que carrega `art_id` no schema,
/// então uma escolha aqui já vale pra toda faixa dele, do jeito que o
/// usuário espera de "trocar a capa". Devolve `None` sem gravar nada se
/// `blob` não for uma imagem decodificável.
pub fn set_album_art(
    db: &Db,
    cache: &ArtCache,
    track: crate::model::TrackId,
    blob: &[u8],
) -> Result<Option<ArtRef>> {
    let Some(art) = cache.store(blob) else {
        return Ok(None);
    };

    db.conn().execute(
        "INSERT INTO cover_art (blob_hash, mime, width, height) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (blob_hash) DO NOTHING",
        rusqlite::params![art.hash.as_slice(), "image/jpeg", art.width, art.height],
    )?;
    db.conn().execute(
        "UPDATE album SET art_id = (
             SELECT id FROM cover_art WHERE blob_hash = ?1
         )
         WHERE id = (SELECT album_id FROM track WHERE id = ?2)",
        rusqlite::params![art.hash.as_slice(), track.0],
    )?;
    Ok(Some(art))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::library::{Sort, view};
    use crate::scan::scan;
    use crate::testutil::{ambiente, escreve};

    /// Duas faixas do mesmo álbum, pra provar que `set_album_art` muda a
    /// capa das duas de uma vez — é o álbum que carrega `art_id`, não a
    /// faixa.
    fn album_com_duas_faixas(nome: &str) -> (Db, crate::testutil::Ambiente) {
        let env = ambiente(nome);
        escreve(&env.musica.join("a/01.mp3"), "Um", "Artista", "Álbum", None);
        escreve(
            &env.musica.join("a/02.mp3"),
            "Dois",
            "Artista",
            "Álbum",
            None,
        );
        let mut db = Db::open_in_memory().expect("abrir");
        let cache = ArtCache::new(env.cache.clone());
        scan(&mut db, &env.musica, &cache).expect("escanear");
        (db, env)
    }

    #[test]
    fn set_album_art_muda_a_capa_das_duas_faixas() {
        let (db, env) = album_com_duas_faixas("set-art");
        let ids = view(&db, Sort::ArtistAlbum).expect("view");

        let cache = ArtCache::new(env.cache);
        let blob = png_valido();
        let art = set_album_art(&db, &cache, ids[0], &blob)
            .expect("gravar capa")
            .expect("png válido");

        let hashes: Vec<Option<Vec<u8>>> = ids
            .iter()
            .map(|id| {
                db.conn()
                    .query_row(
                        "SELECT ca.blob_hash FROM track t
                         JOIN album a ON a.id = t.album_id
                         JOIN cover_art ca ON ca.id = a.art_id
                         WHERE t.id = ?1",
                        [id.0],
                        |r| r.get(0),
                    )
                    .ok()
            })
            .collect();

        assert_eq!(hashes.len(), 2);
        assert!(
            hashes
                .iter()
                .all(|h| h.as_deref() == Some(art.hash.as_slice()))
        );
    }

    #[test]
    fn set_album_art_em_faixa_sem_album_nao_quebra() {
        let (db, env) = album_com_duas_faixas("set-art-sem-album");
        let cache = ArtCache::new(env.cache);

        db.conn()
            .execute("UPDATE track SET album_id = NULL", [])
            .expect("desassociar álbum");

        let ids = view(&db, Sort::ArtistAlbum).expect("view");
        // Não deve dar erro, só não afeta linha nenhuma.
        set_album_art(&db, &cache, ids[0], &png_valido()).expect("não deveria falhar");
    }

    #[test]
    fn set_album_art_com_blob_invalido_devolve_none_sem_gravar() {
        let (db, env) = album_com_duas_faixas("set-art-invalido");
        let cache = ArtCache::new(env.cache);
        let ids = view(&db, Sort::ArtistAlbum).expect("view");

        let resultado = set_album_art(&db, &cache, ids[0], b"isto nao e uma imagem")
            .expect("não deveria falhar");
        assert!(resultado.is_none());
    }

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

    /// Dois workers materializando a mesma capa ao mesmo tempo não podem
    /// brigar pelo arquivo temporário.
    #[test]
    fn duas_gravacoes_simultaneas_da_mesma_capa_nao_colidem() {
        let dir = tmpdir("corrida");
        let blob = png_valido();
        let hash: [u8; 32] = blake3::hash(&blob).into();

        std::thread::scope(|scope| {
            for _ in 0..4 {
                let dir = dir.clone();
                let blob = blob.clone();
                scope.spawn(move || {
                    // Cada thread com o seu cache: nenhuma vê o registro da
                    // outra, então todas decodificam e gravam.
                    ArtCache::new(dir).store(&blob).expect("materializar");
                });
            }
        });

        assert!(ArtRef::thumb_path(&dir, &hash, 96).is_file());
        let sobras: Vec<_> = fs::read_dir(dir.join("art").join(&hex32(&hash)[..2]))
            .expect("ler diretório")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(sobras.is_empty(), "sobrou arquivo temporário: {sobras:?}");
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
