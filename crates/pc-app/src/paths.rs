//! Onde o índice e o cache de capas moram.
//!
//! Nada disso aparece pro usuário: ele aponta uma pasta de música e pronto.

use std::path::PathBuf;

use directories::ProjectDirs;

pub struct Paths {
    /// O índice SQLite.
    pub db: PathBuf,
    /// Raiz do cache de miniaturas. É descartável: apagar só custa um rescan.
    pub cache: PathBuf,
}

impl Paths {
    /// Resolve os diretórios do sistema e garante que existam.
    pub fn resolve() -> std::io::Result<Self> {
        let dirs = ProjectDirs::from("", "", "Yasmine").ok_or_else(|| {
            std::io::Error::other("não foi possível descobrir os diretórios do usuário")
        })?;

        std::fs::create_dir_all(dirs.data_dir())?;
        std::fs::create_dir_all(dirs.cache_dir())?;

        Ok(Self {
            db: dirs.data_dir().join("library.db"),
            cache: dirs.cache_dir().to_path_buf(),
        })
    }
}
