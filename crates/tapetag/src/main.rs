mod align;
mod archive_org;
mod artwork;
mod browser;
mod manifest;
mod parser;
mod scanner;
mod tagger;

use std::path::PathBuf;

use clap::Parser as ClapParser;

#[derive(ClapParser)]
#[command(name = "tapetag", about = "GD/JGB metadata fixer — archive.org alignment")]
struct Cli {
    /// Root directory containing show folders
    #[arg(short, long, default_value = "/Volumes/T5/GD")]
    dir: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let dir = if cli.dir.starts_with("~") {
        let home = dirs_home();
        home.join(cli.dir.strip_prefix("~").unwrap())
    } else {
        cli.dir
    };

    if !dir.is_dir() {
        anyhow::bail!("Directory not found: {}", dir.display());
    }

    browser::run(&dir)
}

fn dirs_home() -> PathBuf {
    directories::BaseDirs::new()
        .expect("Could not determine home directory")
        .home_dir()
        .to_path_buf()
}
