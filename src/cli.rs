//! Command-line interface.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ankigen",
    version,
    about = "Compile text flashcards into an Anki .apkg"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Parse sources, assign ids, write them back, and emit a .apkg.
    Build(BuildArgs),
}

#[derive(Args)]
pub struct BuildArgs {
    /// Card files and/or directories (directories are recursed for .md/.cards).
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,

    /// Output .apkg path.
    #[arg(short, long, default_value = "deck.apkg")]
    pub output: PathBuf,

    /// Prefix prepended above the directory-derived decks.
    #[arg(long)]
    pub deck: Option<String>,

    /// Parse + validate only: write nothing (no @id, no deck-id.txt, no .apkg).
    #[arg(long)]
    pub check: bool,

    /// Build + emit but do not persist ids back into sources.
    #[arg(long = "no-write-back")]
    pub no_write_back: bool,
}
