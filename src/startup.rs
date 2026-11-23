use crate::brain::runtime::BrainRuntime;
use crate::config::{ArchonConfig, BrainConfig, LLMConfig, ServerConfig, default_embedding_model};
use reqwest::Url;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Duration, interval};
use tracing::{error, info};

pub struct StartupMilestone;

impl StartupMilestone {
    pub const CONFIG: &'static str = "config";
    pub const VARDADB_BOOT: &'static str = "vardadb_boot";
    pub const VARDADB_HTTP: &'static str = "vardadb_http";
    pub const BRAIN_HTTP: &'static str = "brain_http";
    pub const DEV_SESSION: &'static str = "dev_session";
}

#[derive(Clone, Default)]
pub struct StartupContext {
    server_state: Arc<Mutex<Option<Arc<vardadb::ServerState>>>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl StartupContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_server_state(&self, server_state: Arc<vardadb::ServerState>) {
        let mut slot = self
            .server_state
            .lock()
            .expect("startup context server_state poisoned");
        *slot = Some(server_state);
    }

    pub fn server_state(&self) -> Option<Arc<vardadb::ServerState>> {
        self.server_state
            .lock()
            .expect("startup context server_state poisoned")
            .clone()
    }

    pub fn set_failure(&self, message: String) {
        let mut slot = self
            .failure
            .lock()
            .expect("startup context failure poisoned");
        *slot = Some(message);
    }

    pub fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .expect("startup context failure poisoned")
            .clone()
    }
}

pub struct StartupArtifacts {
    pub server_state: Arc<vardadb::ServerState>,
}

#[derive(Debug)]
pub struct StartupError {
    milestone_key: &'static str,
    message: String,
}

impl StartupError {
    fn new(milestone_key: &'static str, message: impl Into<String>) -> Self {
        Self {
            milestone_key,
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for StartupError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.milestone_key, self.message)
    }
}

impl std::error::Error for StartupError {}

pub fn build_startup_config(dev_mode: bool) -> rust_cef::DeferredStartupConfig {
    let mut milestones = vec![
        rust_cef::MilestoneDefinition {
            key: StartupMilestone::CONFIG.to_string(),
            label: "Loading configuration".to_string(),
            weight: 5,
            required: true,
        },
        rust_cef::MilestoneDefinition {
            key: StartupMilestone::VARDADB_BOOT.to_string(),
            label: "Starting VardaDB".to_string(),
            weight: 30,
            required: true,
        },
        rust_cef::MilestoneDefinition {
            key: StartupMilestone::VARDADB_HTTP.to_string(),
            label: "Starting database server".to_string(),
            weight: 10,
            required: true,
        },
        rust_cef::MilestoneDefinition {
            key: StartupMilestone::BRAIN_HTTP.to_string(),
            label: "Starting Archon brain".to_string(),
            weight: if dev_mode { 40 } else { 55 },
            required: true,
        },
    ];

    if dev_mode {
        milestones.push(rust_cef::MilestoneDefinition {
            key: StartupMilestone::DEV_SESSION.to_string(),
            label: "Preparing development session".to_string(),
            weight: 15,
            required: true,
        });
    }

    let coordinator =
        rust_cef::StartupCoordinator::new(milestones).expect("startup milestones should be valid");

    rust_cef::DeferredStartupConfig {
        coordinator,
        ui: rust_cef::StartupUiConfig {
            title: "Archon".to_string(),
            subtitle: Some("Starting application".to_string()),
            ..Default::default()
        },
        transition_delay_ms: 350,
    }
}

pub fn spawn_startup_worker(
    context: StartupContext,
    coordinator: rust_cef::StartupCoordinator,
    config_path: String,
    dev_mode: bool,
) {
    std::thread::spawn(move || {
        info!("Starting Archon startup worker thread...");
        let runtime = Runtime::new().expect("Failed to create Tokio runtime for startup worker");

        match runtime.block_on(run_startup_sequence(
            context.clone(),
            coordinator.clone(),
            &config_path,
            dev_mode,
        )) {
            Ok(artifacts) => {
                context.set_server_state(artifacts.server_state);
                runtime.block_on(async {
                    std::future::pending::<()>().await;
                });
            }
            Err(err) => {
                let message = err.to_string();
                context.set_failure(message.clone());
                error!("Startup failed: {}", message);
                eprintln!("Archon startup failed: {}", message);
                std::thread::sleep(std::time::Duration::from_millis(150));
                std::process::exit(1);
            }
        }
    });
}

pub fn load_varda_config(
    config_path: &str,
) -> Result<vardadb::config::VardaConfig, Box<dyn std::error::Error>> {
    let mut config = vardadb::config::VardaConfig::load_from_file(config_path)?;
    let config_content = std::fs::read_to_string(config_path)?;
    let raw_config: toml::Value = toml::from_str(&config_content)?;

    let nested_hf_token = raw_config
        .get("llm")
        .and_then(|llm| llm.get("huggingface"))
        .and_then(|hf| hf.get("hf_token"))
        .and_then(|token| token.as_str())
        .map(str::to_owned);

    let legacy_hf_token = raw_config
        .get("huggingface")
        .and_then(|hf| hf.get("hf_token"))
        .and_then(|token| token.as_str())
        .map(str::to_owned);

    if let Some(hf_token) = nested_hf_token.or(legacy_hf_token) {
        config.llm.huggingface.hf_token = Some(hf_token);
    }

    Ok(config)
}

pub async fn run_startup_sequence(
    context: StartupContext,
    coordinator: rust_cef::StartupCoordinator,
    config_path: &str,
    dev_mode: bool,
) -> Result<StartupArtifacts, StartupError> {
    let milestone = milestone_id(&coordinator, StartupMilestone::CONFIG)?;
    coordinator_start(&coordinator, milestone, StartupMilestone::CONFIG)?;
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::CONFIG, 0.2)?;
    let varda_config = load_varda_config(config_path)
        .map_err(|err| fail_milestone(&coordinator, StartupMilestone::CONFIG, err.to_string()))?;
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::CONFIG, 0.55)?;
    let raw_config = load_raw_config(config_path)
        .map_err(|err| fail_milestone(&coordinator, StartupMilestone::CONFIG, err.to_string()))?;
    let archon_config = build_archon_config(&varda_config, &raw_config);
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::CONFIG, 0.85)?;
    coordinator_complete(&coordinator, milestone, StartupMilestone::CONFIG)?;

    let milestone = milestone_id(&coordinator, StartupMilestone::VARDADB_BOOT)?;
    coordinator_start(&coordinator, milestone, StartupMilestone::VARDADB_BOOT)?;
    let vardadb_boot_progress =
        MilestoneProgress::spawn(coordinator.clone(), milestone, 0.92, 0.06, 70);
    let (server_state, db_app) = vardadb::init_system(varda_config.clone()).await;
    vardadb_boot_progress.stop().await;
    coordinator_complete(&coordinator, milestone, StartupMilestone::VARDADB_BOOT)?;

    let milestone = milestone_id(&coordinator, StartupMilestone::VARDADB_HTTP)?;
    coordinator_start(&coordinator, milestone, StartupMilestone::VARDADB_HTTP)?;
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::VARDADB_HTTP, 0.2)?;
    let db_port = varda_config.server.port;
    let db_addr = format!("0.0.0.0:{}", db_port);
    let db_listener = tokio::net::TcpListener::bind(&db_addr)
        .await
        .map_err(|err| {
            fail_milestone(
                &coordinator,
                StartupMilestone::VARDADB_HTTP,
                err.to_string(),
            )
        })?;
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::VARDADB_HTTP, 0.7)?;
    tokio::spawn(async move {
        if let Err(err) = axum::serve(db_listener, db_app).await {
            error!("VardaDB Server Error: {}", err);
        }
    });
    info!("VardaDB running at http://127.0.0.1:{}", db_port);
    coordinator_complete(&coordinator, milestone, StartupMilestone::VARDADB_HTTP)?;

    let brain_runtime = BrainRuntime::new(&archon_config, Arc::new(Mutex::new(None)));

    let milestone = milestone_id(&coordinator, StartupMilestone::BRAIN_HTTP)?;
    coordinator_start(&coordinator, milestone, StartupMilestone::BRAIN_HTTP)?;
    let brain_http_progress =
        MilestoneProgress::spawn(coordinator.clone(), milestone, 0.94, 0.05, 85);
    let brain_app = crate::brain::start_service_with_runtime(
        server_state.clone(),
        archon_config.clone(),
        brain_runtime.clone(),
    )
    .await;
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::BRAIN_HTTP, 0.75)?;
    let brain_port = 9000;
    let brain_addr = format!("0.0.0.0:{}", brain_port);
    let brain_listener = tokio::net::TcpListener::bind(&brain_addr)
        .await
        .map_err(|err| {
            fail_milestone(&coordinator, StartupMilestone::BRAIN_HTTP, err.to_string())
        })?;
    coordinator_set_progress(&coordinator, milestone, StartupMilestone::BRAIN_HTTP, 0.9)?;
    tokio::spawn(async move {
        if let Err(err) = axum::serve(brain_listener, brain_app).await {
            error!("Brain Server Error: {}", err);
        }
    });
    info!("Archon Brain running at http://127.0.0.1:{}", brain_port);
    brain_http_progress.stop().await;
    coordinator_complete(&coordinator, milestone, StartupMilestone::BRAIN_HTTP)?;

    if dev_mode {
        let milestone = milestone_id(&coordinator, StartupMilestone::DEV_SESSION)?;
        coordinator_start(&coordinator, milestone, StartupMilestone::DEV_SESSION)?;
        let dev_progress =
            MilestoneProgress::spawn(coordinator.clone(), milestone, 0.94, 0.08, 100);
        wait_for_dev_server("http://localhost:5173", Duration::from_secs(60))
            .await
            .map_err(|err| fail_milestone(&coordinator, StartupMilestone::DEV_SESSION, err))?;
        dev_progress.stop().await;
        coordinator_complete(&coordinator, milestone, StartupMilestone::DEV_SESSION)?;
    }

    context.set_server_state(server_state.clone());
    coordinator
        .mark_ready_for_cef()
        .map_err(|err| StartupError::new("cef_gate", err.to_string()))?;

    Ok(StartupArtifacts { server_state })
}

fn load_raw_config(config_path: &str) -> Result<toml::Value, StartupError> {
    let config_content = std::fs::read_to_string(config_path)
        .map_err(|err| StartupError::new(StartupMilestone::CONFIG, err.to_string()))?;
    toml::from_str(&config_content)
        .map_err(|err| StartupError::new(StartupMilestone::CONFIG, err.to_string()))
}

fn build_archon_config(
    db_config: &vardadb::config::VardaConfig,
    raw_config: &toml::Value,
) -> ArchonConfig {
    let brain_server_config = ServerConfig {
        port: db_config.server.port,
        storage_path: db_config.server.storage_path.clone(),
    };

    let brain_llm_config = LLMConfig {
        base_url: format!("http://localhost:{}", db_config.server.port),
        model: String::new(),
        api_key: None,
        embedding_model: default_embedding_model(),
    };

    let brain_config = if let Some(brain_table) = raw_config.get("brain") {
        let context_window_size = brain_table
            .get("context_window_size")
            .and_then(|value| value.as_integer())
            .unwrap_or(2) as usize;
        let planning = brain_table
            .get("planning")
            .and_then(|value| value.as_str())
            .map(String::from);
        let thinking = brain_table
            .get("thinking")
            .and_then(|value| value.as_str())
            .map(String::from);
        BrainConfig {
            context_window_size,
            planning,
            thinking,
        }
    } else {
        BrainConfig::default()
    };

    ArchonConfig {
        server: brain_server_config,
        llm: brain_llm_config,
        fast_llm: None,
        brain: brain_config,
    }
}

fn milestone_id(
    coordinator: &rust_cef::StartupCoordinator,
    key: &'static str,
) -> Result<rust_cef::MilestoneId, StartupError> {
    coordinator
        .milestone_id(key)
        .ok_or_else(|| StartupError::new(key, "missing startup milestone"))
}

fn coordinator_start(
    coordinator: &rust_cef::StartupCoordinator,
    milestone: rust_cef::MilestoneId,
    key: &'static str,
) -> Result<(), StartupError> {
    coordinator
        .start(milestone)
        .map_err(|err| StartupError::new(key, err.to_string()))
}

fn coordinator_complete(
    coordinator: &rust_cef::StartupCoordinator,
    milestone: rust_cef::MilestoneId,
    key: &'static str,
) -> Result<(), StartupError> {
    coordinator
        .complete(milestone)
        .map_err(|err| StartupError::new(key, err.to_string()))
}

fn coordinator_set_progress(
    coordinator: &rust_cef::StartupCoordinator,
    milestone: rust_cef::MilestoneId,
    key: &'static str,
    progress: f32,
) -> Result<(), StartupError> {
    coordinator
        .set_progress(milestone, progress)
        .map_err(|err| StartupError::new(key, err.to_string()))
}

fn fail_milestone(
    coordinator: &rust_cef::StartupCoordinator,
    key: &'static str,
    message: String,
) -> StartupError {
    if let Some(milestone) = coordinator.milestone_id(key) {
        let _ = coordinator.fail(milestone, message.clone());
    }
    StartupError::new(key, message)
}

async fn wait_for_dev_server(target_url: &str, timeout: Duration) -> Result<(), String> {
    let url = Url::parse(target_url)
        .map_err(|err| format!("invalid dev server URL `{target_url}`: {err}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("dev server URL missing host: {target_url}"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| format!("dev server URL missing port: {target_url}"))?;

    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if tokio::net::TcpStream::connect((host, port)).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    Err(format!(
        "timed out waiting for dev server at {target_url} after {}s",
        timeout.as_secs()
    ))
}

struct MilestoneProgress {
    stop_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl MilestoneProgress {
    fn spawn(
        coordinator: rust_cef::StartupCoordinator,
        milestone: rust_cef::MilestoneId,
        max_progress: f32,
        step: f32,
        tick_ms: u64,
    ) -> Self {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut progress = step.min(max_progress);
            let _ = coordinator.set_progress(milestone, progress);

            let mut ticker = interval(Duration::from_millis(tick_ms));
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        progress = (progress + step).min(max_progress);
                        let _ = coordinator.set_progress(milestone, progress);
                        if progress >= max_progress {
                            break;
                        }
                    }
                    _ = &mut stop_rx => break,
                }
            }
        });

        Self {
            stop_tx: Some(stop_tx),
            task,
        }
    }

    async fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        let _ = self.task.await;
    }
}
