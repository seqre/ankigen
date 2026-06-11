use clap::Parser;

fn main() -> miette::Result<()> {
    let cli = ankigen::cli::Cli::parse();
    ankigen::run(cli)?;
    Ok(())
}
