//! Utilidades compartilhadas entre os testes do crate.
//!
//! Os arquivos gerados aqui são MP3 válidos — frames MPEG-1 Layer III de
//! silêncio com tag ID3v2 de verdade. Testar o scanner contra bytes falsos
//! testaria o `lofty` errado.

use std::fs;
use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::Accessor;
use lofty::tag::{Tag, TagExt, TagType};

const FRAME_LEN: usize = 417;
const FRAME_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];

/// ~1 segundo de silêncio em frames válidos.
pub fn mp3_silencioso() -> Vec<u8> {
    let mut out = vec![0u8; 39 * FRAME_LEN];
    for frame in out.as_chunks_mut::<FRAME_LEN>().0 {
        frame[..4].copy_from_slice(&FRAME_HEADER);
    }
    out
}

/// PNG 8x8 de cor sólida, para servir de capa.
pub fn png(cor: [u8; 3]) -> Vec<u8> {
    let mut img = image::RgbImage::new(8, 8);
    for p in img.pixels_mut() {
        *p = image::Rgb(cor);
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("codificar png");
    out
}

/// Grava um MP3 com tag no caminho dado, criando os diretórios.
pub fn escreve(path: &Path, titulo: &str, artista: &str, album: &str, capa: Option<&[u8]>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("criar diretório");
    }
    fs::write(path, mp3_silencioso()).expect("gravar áudio");

    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(titulo.to_owned());
    tag.set_artist(artista.to_owned());
    tag.set_album(album.to_owned());
    tag.set_track(1);
    if let Some(capa) = capa {
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            capa.to_vec(),
        ));
    }
    tag.save_to_path(path, WriteOptions::default())
        .expect("gravar tag");
}

/// Roda um `SELECT count(*)` e devolve o número.
pub fn conta(db: &crate::Db, sql: &str) -> i64 {
    db.conn()
        .query_row(sql, [], |r| r.get(0))
        .expect("consultar contagem")
}

/// Pasta de música + pasta de cache, limpas, com nome próprio por teste (os
/// testes rodam em paralelo).
pub struct Ambiente {
    pub musica: PathBuf,
    pub cache: PathBuf,
}

pub fn ambiente(nome: &str) -> Ambiente {
    let dir = std::env::temp_dir().join(format!("player-test-{nome}"));
    let _ = fs::remove_dir_all(&dir);
    let musica = dir.join("musica");
    let cache = dir.join("cache");
    fs::create_dir_all(&musica).expect("criar pasta de música");
    Ambiente { musica, cache }
}
