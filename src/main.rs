pub mod brain;
pub mod config;
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
    /// Fetch chapters into output/gospels (live fetching requires Chromium and PIA_USER / PIA_PASS)
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
    ScriptureImport {
        /// Override scripture-import.input_dir from config.toml
        #[arg(long)]
        input: Option<std::path::PathBuf>,
    },
    /// Database Management
    #[command(subcommand)]
    Db(vardadb::cli::DbCommands),
}

#[derive(RustEmbed)]
#[folder = "frontend/dist"] // Point this to your built UI assets
struct Assets;

fn run_command(command: Commands) -> anyhow::Result<()> {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("output");
    match command {
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
        Commands::ScriptureImport { input } => {
            let config = startup::load_varda_config("config.toml")
                .map_err(|e| anyhow::anyhow!("Failed to load config.toml: {e}"))?;
            let storage = std::path::Path::new(&config.server.storage_path);
            let destination = storage.join(archon_scripture::DATABASE_FILENAME);
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let input = match input {
                Some(path) => path,
                None => {
                    let text = std::fs::read_to_string("config.toml")?;
                    let config: toml::Value = toml::from_str(&text)?;
                    let path = config
                        .get("scripture-import")
                        .and_then(|section| section.get("input_dir"))
                        .and_then(toml::Value::as_str)
                        .filter(|path| !path.trim().is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!("Set scripture-import.input_dir in config.toml")
                        })?;
                    std::path::PathBuf::from(path)
                }
            };
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
                .prefix(".scripture-import-")
                .tempdir_in(&output)?;
            let rows = archon_scripture::import(&input, ingest.path(), storage)?;
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

fn main() {
    // 1. Initialize Logging (Basic)
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,archon=debug,fjall=warn,lsm_tree=warn".into());

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

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
