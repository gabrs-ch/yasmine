//! `yasmine-sync-host` — o lado PC do sync.
//!
//! Abre a `library.db` do Yasmine, mostra um QR de pareamento, e serve a
//! biblioteca pro celular que ler esse QR. Sem GUI é só terminal (o QR sai em
//! blocos unicode); com `--gui` (build `--features gui`) abre uma janela com o
//! QR grande e o progresso.
//!
//! ```text
//! yasmine-sync-host --music ~/Musica            # primeira vez: aponta a pasta
//! yasmine-sync-host                             # depois: usa o índice que já existe
//! yasmine-sync-host --gui --mdns                # janela + anúncio na LAN
//! ```

// Binário pequeno: passar `Options` por valor pras funções de entrada é
// idiomático aqui, não vale o ruído do lint.
#![allow(clippy::needless_pass_by_value)]

mod host;

#[cfg(feature = "gui")]
mod gui;

use std::process::ExitCode;

use host::Options;

fn main() -> ExitCode {
    let opts = match Options::from_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}\n\n{}", Options::USAGE);
            return ExitCode::FAILURE;
        }
    };

    let result = if opts.gui {
        #[cfg(feature = "gui")]
        {
            gui::run(opts)
        }
        #[cfg(not(feature = "gui"))]
        {
            Err("compilado sem GUI — rode `cargo build --features gui` ou tire o --gui".into())
        }
    } else {
        host::run_cli(opts)
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}
