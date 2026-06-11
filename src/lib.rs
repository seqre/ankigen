//! `ankigen` — compile simple text flashcards into an Anki `.apkg`, with stable
//! per-card identity so edits update existing cards instead of duplicating them.
//!
//! This is primarily a library; the `ankigen` binary is a thin wrapper over
//! [`run`].

// `AnkigenError` carries source text + spans for rich diagnostics, which makes
// it larger than clippy likes. The `Err` path is cold in a batch CLI, so the
// size is irrelevant; boxing it everywhere would only add noise.
#![allow(clippy::result_large_err)]

pub mod anki;
pub mod cli;
pub mod deck;
pub mod error;
pub mod id;
pub mod media;
pub mod model;
pub mod pipeline;
pub mod render;
pub mod source;
pub mod util;

pub use error::{AnkigenError, Result};
pub use model::{CardKindSpec, CardSpec, ModelKey, ParsedCard, ParsedFile};
pub use source::{CardSource, CardsSource};

/// Run the CLI: dispatch the chosen subcommand.
pub fn run(cli: cli::Cli) -> Result<()> {
    match cli.command {
        cli::Command::Build(args) => {
            let opts = pipeline::BuildOptions {
                output: args.output,
                deck_prefix: args.deck,
                check: args.check,
                write_back: !args.no_write_back,
            };
            pipeline::build(&args.paths, &opts)
        }
    }
}
