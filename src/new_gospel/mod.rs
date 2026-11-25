mod compare;
mod export;
mod ingest;
mod marks;

#[derive(clap::Subcommand)]
pub enum Command {
    Import(ingest::Cli),
    Export(export::Cli),
    Compare(compare::Cli),
    /// Back up or restore saved verse marks
    #[command(subcommand)]
    Marks(marks::Command),
}

pub fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Import(args) => ingest::run(args),
        Command::Export(args) => export::run(args),
        Command::Compare(args) => compare::run(args),
        Command::Marks(command) => marks::run(command),
    }
}

fn read_settings() -> anyhow::Result<toml::Value> {
    use anyhow::Context;

    let content = std::fs::read_to_string("config.toml").context("reading config.toml")?;
    toml::from_str(&content).context("parsing config.toml")
}

fn configured_path(
    settings: &toml::Value,
    keys: &[&str],
) -> anyhow::Result<Option<std::path::PathBuf>> {
    use anyhow::Context;

    let Some(value) = keys
        .iter()
        .try_fold(settings, |parent, key| parent.get(*key))
    else {
        return Ok(None);
    };
    let path = value
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .with_context(|| format!("Set {} in config.toml to a nonempty path", keys.join(".")))?;
    Ok(Some(path.into()))
}
