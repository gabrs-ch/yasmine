//! `art://localhost/<hex-hash>/<size>` → os bytes JPEG da miniatura no cache.
//!
//! Substitui o carregador de textura do egui: no webview a capa é uma `<img>`
//! comum, e o Rust serve o arquivo que o scan já gerou (`ArtRef::thumb_path`,
//! tamanhos 96 e 512).

// A forma do handler é ditada por `register_uri_scheme_protocol`: contexto e
// `Request` entram por valor.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};

use tauri::http::{Request, Response, StatusCode};
use tauri::{Runtime, UriSchemeContext};

use player_core::ArtRef;

use crate::hexhash;

/// Tamanhos de miniatura que o scan grava. Pedir outro → 404.
const SIZES: [u32; 2] = [96, 512];

pub fn handle<R: Runtime>(
    cache_dir: &Path,
    _ctx: UriSchemeContext<'_, R>,
    req: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let not_found = || {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new())
            .expect("resposta 404 é válida")
    };

    // path = "/<hex>/<size>"
    let path = req.uri().path();
    let mut segs = path.trim_start_matches('/').split('/');
    let (Some(hex), Some(size_str), None) = (segs.next(), segs.next(), segs.next()) else {
        return not_found();
    };
    let (Some(hash), Ok(size)) = (hexhash::decode(hex), size_str.parse::<u32>()) else {
        return not_found();
    };
    if !SIZES.contains(&size) {
        return not_found();
    }

    match std::fs::read(ArtRef::thumb_path(cache_dir, &hash, size)) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/jpeg")
            // As miniaturas são imutáveis por hash: o webview pode cachear à
            // vontade dentro da sessão.
            .header("Cache-Control", "max-age=31536000, immutable")
            .body(bytes)
            .expect("resposta 200 com corpo é válida"),
        Err(_) => not_found(),
    }
}

/// Fecha o `cache_dir` num closure `Fn` pro `register_uri_scheme_protocol`.
pub fn handler<R: Runtime>(
    cache_dir: PathBuf,
) -> impl Fn(UriSchemeContext<'_, R>, Request<Vec<u8>>) -> Response<Vec<u8>> + Send + Sync + 'static
{
    move |ctx, req| handle(&cache_dir, ctx, req)
}
