pub mod brain;
pub mod config;
mod new_gospel;
pub mod startup;

use clap::{Parser, Subcommand};
use rust_embed::RustEmbed;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "archon",
    about = "Archon desktop app and command-line tools",
    after_help = "Examples:\n  cargo run --\n  cargo run -- --dev\n  cargo run -- scrape --dry-run --translation T4T --book GEN --chapter 1\n  cargo run -- scrape --translation T4T --book GEN --chapter 1\n  cargo run -- json --translation T4T\n  cargo run -- db --help\n  cargo run -- scrape --help\n  cargo run -- json --help"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in dev mode
    #[arg(long)]
    dev: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch chapters into output/gospels (PIA credentials can come from crates/pia_proxy/.env)
    Scrape {
        /// Preview URLs and output paths offline
        #[arg(long)]
        dry_run: bool,
        /// Configured target name or zero-based index
        #[arg(long)]
        target: Option<String>,
        /// Translation codes, comma-separated
        #[arg(long, value_delimiter = ',')]
        translation: Vec<String>,
        /// USFM book IDs, comma-separated
        #[arg(long, value_delimiter = ',')]
        book: Vec<String>,
        /// Chapter numbers, comma-separated
        #[arg(long, value_delimiter = ',', value_parser = clap::value_parser!(u32).range(1..))]
        chapter: Vec<u32>,
        /// Language code
        #[arg(long)]
        lang: Option<String>,
    },
    /// Convert saved HTML from output/gospels into chapter JSON under output/json
    Json {
        /// Translation codes, comma-separated (default: all available)
        #[arg(long, value_delimiter = ',')]
        translation: Vec<String>,
    },
    /// Convert chapter JSON to JSONL and bulk-load a new scripture database
    BibleImport,
    CompareNewGospel(new_gospel::compare::Cli),
    IngestNewGospel(new_gospel::ingest::Cli),
    ExportNewGospel(new_gospel::export::Cli),
    /// Database Management
    #[command(subcommand)]
    Db(vardadb::cli::DbCommands),
}

#[derive(RustEmbed)]
#[folder = "frontend/dist"] // Point this to your built UI assets
struct Assets;

fn bible_input_dir(config: &toml::Value) -> anyhow::Result<&str> {
    config
        .get("bible-import")
        .or_else(|| config.get("scripture-import"))
        .and_then(|section| section.get("input_dir"))
        .and_then(toml::Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Set bible-import.input_dir in config.toml"))
}

fn run_command(command: Commands) -> anyhow::Result<()> {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("output");
    match command {
        Commands::CompareNewGospel(args) => new_gospel::compare::run(args)?,
        Commands::IngestNewGospel(args) => new_gospel::ingest::run(args)?,
        Commands::ExportNewGospel(args) => new_gospel::export::run(args)?,
        Commands::Db(command) => {
            let config = startup::load_varda_config("config.toml")
                .map_err(|e| anyhow::anyhow!("Failed to load config.toml: {e}"))?;
            vardadb::cli::handle_db_command_direct(&command, &config)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Commands::Scrape {
            dry_run,
            target,
            translation,
            book,
            chapter,
            lang,
        } => {
            let filters = archon_scraper::Narrow {
                translations: translation,
                books: book,
                chapters: chapter,
                language: lang,
            };
            if dry_run {
                print!(
                    "{}",
                    archon_scraper::preview(&output, target.as_deref(), &filters)?
                );
            } else {
                let report = tokio::runtime::Runtime::new()?.block_on(archon_scraper::run(
                    &output,
                    target.as_deref(),
                    &filters,
                ))?;
                println!("{}", report.summary());
                for failure in &report.failed {
                    eprintln!("{}: {}", failure.url, failure.error);
                }
                anyhow::ensure!(
                    report.failed.is_empty(),
                    "{} chapter(s) failed",
                    report.failed.len()
                );
            }
        }
        Commands::BibleImport => {
            let config = startup::load_varda_config("config.toml")
                .map_err(|e| anyhow::anyhow!("Failed to load config.toml: {e}"))?;
            let storage = std::path::Path::new(&config.server.storage_path);
            let destination = storage.join(archon_scripture::DATABASE_FILENAME);
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let text = std::fs::read_to_string("config.toml")?;
            let settings: toml::Value = toml::from_str(&text)?;
            let input = std::path::Path::new(bible_input_dir(&settings)?);
            let name = archon_scripture::DATABASE_NAME;

            let varda = vardadb::storage::backend::Storage::new(storage, config.server.node_id)?;
            let registered = varda
                .list_databases()
                .into_iter()
                .find(|(db, _)| db == name);
            if let Some((_, path)) = &registered {
                anyhow::ensure!(
                    std::fs::canonicalize(path)? == std::fs::canonicalize(&destination)?,
                    "Database {name} is registered at {path}"
                );
            }
            let schema = root.join("schema/archon.graphql");
            let schema = schema.to_string_lossy().into_owned();
            std::fs::create_dir_all(&output)?;
            let ingest = tempfile::Builder::new()
                .prefix(".bible-import-")
                .tempdir_in(&output)?;
            let rows = archon_scripture::import(input, ingest.path(), storage)?;
            if registered.is_none() {
                varda.create_database(name)?;
            }
            drop(varda);
            vardadb::cli::handle_db_command_direct(
                &vardadb::cli::DbCommands::Apply {
                    name: name.to_owned(),
                    schema,
                },
                &config,
            )?;
            ingest.close()?;
            println!(
                "Imported {rows} scripture records into {}",
                destination.display()
            );
        }
        Commands::Json { translation } => {
            let selection: Vec<_> = translation.iter().map(String::as_str).collect();
            let outcome = archon_json::run(&output, &selection)?;
            outcome.print_failures();
            println!("{}", outcome.summary("json"));
            anyhow::ensure!(
                outcome.failed.is_empty(),
                "{} chapter(s) failed",
                outcome.failed.len()
            );
        }
    }
    Ok(())
}

struct ChromiumProtocolFilter;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ChromiumProtocolFilter {
    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        !is_chromium_protocol_noise(event)
    }
}

fn is_chromium_protocol_noise(event: &tracing::Event<'_>) -> bool {
    if !matches!(
        event.metadata().target(),
        "chromiumoxide::conn" | "chromiumoxide::handler"
    ) {
        return false;
    }

    #[derive(Default)]
    struct MessageMatch(bool);
    impl tracing::field::Visit for MessageMatch {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                let message = format!("{value:?}");
                self.0 = (message.starts_with("Failed to deserialize WS response ")
                    || message.starts_with("WS Connection error: Serde("))
                    && message.contains("data did not match any variant of untagged enum Message");
            }
        }
    }
    let mut message = MessageMatch::default();
    event.record(&mut message);
    message.0
}

fn main() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    // 1. Initialize Logging (Basic)
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,archon=debug,fjall=warn,lsm_tree=warn".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(ChromiumProtocolFilter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Check if we are a CEF subprocess (e.g. GPU process, Renderer)
    // If so, we should NOT start the database or other heavy backend services.
    let is_subprocess = std::env::args().any(|a| a.starts_with("--type="));

    if is_subprocess {
        info!("CEF Subprocess detected.");
        let app = rust_cef::App::new()
            .title("Archon Helper")
            .assets(|path| Assets::get(path));

        if let Err(e) = app.run() {
            eprintln!("Helper Application error: {}", e);
        }
        return;
    }

    // Parse CLI
    // We use try_parse to catch Help/Version or Errors
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            e.exit();
        }
    };

    // If it's a CLI command, execute and exit
    if let Some(cmd) = cli.command {
        if let Err(error) = run_command(cmd) {
            eprintln!("Command failed: {error:#}");
            std::process::exit(1);
        }
        return;
    }

    // Normal Startup
    info!("Archon Starting...");

    let cwd = std::env::current_dir().unwrap_or_default();
    info!("Current Working Directory: {:?}", cwd);

    let config_path = "config.toml";
    let abs_config_path = cwd.join(config_path);
    info!("Loading config from: {:?}", abs_config_path);

    let startup_context = startup::StartupContext::new();
    let deferred_startup = startup::build_startup_config(cli.dev);
    startup::spawn_startup_worker(
        startup_context.clone(),
        deferred_startup.coordinator.clone(),
        config_path.to_string(),
        cli.dev,
    );

    info!("Starting Rust-CEF App...");
    run_app(cli.dev, startup_context, deferred_startup);
}

fn run_app(
    dev_mode: bool,
    startup_context: startup::StartupContext,
    deferred_startup: rust_cef::DeferredStartupConfig,
) {
    let app = rust_cef::App::new()
        .title("Archon")
        .assets(|path| Assets::get(path))
        .deferred_startup(deferred_startup)
        .visible(false)
        .on_exit(move || {
            println!("\n[Archon] Window closed. Flushing VardaDB before shutdown...");
            if let Some(state) = startup_context.server_state() {
                if let Err(e) = state.storage.flush() {
                    eprintln!("Failed to flush VardaDB on exit: {}", e);
                } else {
                    println!("[Archon] VardaDB flushed successfully.");
                }
            } else if let Some(failure) = startup_context.failure() {
                eprintln!("Startup failed before shutdown handling: {}", failure);
            } else {
                println!("[Archon] No VardaDB state was ready at shutdown.");
            }
        });

    // Configure Dev Workflow
    let app = if dev_mode {
        info!("Running in DEV MODE");
        app.dev_config(rust_cef::DevConfig {
            command: "bun dev".to_string(),
            url: "http://localhost:5173".to_string(),
            cwd: Some("frontend".to_string()),
            open_devtools: false,
        })
        .on_ready(|handle| {
            // In dev mode, show window when ready
            handle.show();
        })
    } else {
        app.on_ready(|handle| {
            handle.show();
        })
    };

    if let Err(e) = app.run() {
        eprintln!("Application error: {}", e);
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn bible_import_config_precedence() {
        let legacy = "[scripture-import]\ninput_dir = 'old'";
        let current = "[bible-import]\ninput_dir = 'new'";
        for (text, expected) in [
            (legacy.to_owned(), "old"),
            (current.to_owned(), "new"),
            (format!("{legacy}\n{current}"), "new"),
        ] {
            assert_eq!(
                bible_input_dir(&toml::from_str(&text).unwrap()).unwrap(),
                expected
            );
        }
    }
}
