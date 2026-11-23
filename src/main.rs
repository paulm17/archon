pub mod brain;
pub mod config;
pub mod startup;

use clap::{Parser, Subcommand};
use rust_embed::RustEmbed;
use tracing::info;

#[derive(Parser)]
#[command(name = "archon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in dev mode
    #[arg(long)]
    dev: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Database Management
    #[command(subcommand)]
    Db(vardadb::cli::DbCommands),
}

#[derive(RustEmbed)]
#[folder = "frontend/dist"] // Point this to your built UI assets
struct Assets;

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
        let config_path = "config.toml";
        // Load config for CLI (using VardaConfig directly as CLI expects it)
        let config = match startup::load_varda_config(config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to load config from {}: {}", config_path, e);
                std::process::exit(1);
            }
        };

        match cmd {
            Commands::Db(db_cmd) => {
                if let Err(e) = vardadb::cli::handle_db_command_direct(&db_cmd, &config) {
                    eprintln!("Command failed: {}", e);
                    std::process::exit(1);
                }
            }
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
