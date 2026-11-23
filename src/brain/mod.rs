pub mod error;
pub mod llm;
pub mod models;
pub mod nlp_retrieval;
pub mod planner;
pub mod query_analyzer;
pub mod reasoning;
pub mod routes;
pub mod runtime;
pub mod types;

use crate::brain::llm::LlmClient;
use crate::brain::runtime::BrainRuntime;
use crate::config::ArchonConfig;
use axum::{Router, routing::post};
use fastembed::TextEmbedding;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;
use vardadb::ServerState;

pub struct AppState {
    pub server: Arc<ServerState>,
    pub config: ArchonConfig,
    pub embedding_model: Arc<Mutex<Option<TextEmbedding>>>,
    pub llm_client: Arc<LlmClient>,
    pub planning_client: Option<Arc<LlmClient>>,
}

pub async fn start_service(server_state: Arc<ServerState>, config: ArchonConfig) -> Router {
    let runtime = BrainRuntime::new(&config, Arc::new(Mutex::new(None)));
    start_service_with_runtime(server_state, config, runtime).await
}

pub async fn start_service_with_runtime(
    server_state: Arc<ServerState>,
    config: ArchonConfig,
    runtime: BrainRuntime,
) -> Router {
    let app_state = Arc::new(AppState {
        server: server_state,
        config,
        embedding_model: runtime.embedding_model,
        llm_client: runtime.llm_client,
        planning_client: runtime.planning_client,
    });

    Router::new()
        .route("/ask-ai", post(routes::handle_ask))
        .layer(CorsLayer::permissive())
        .with_state(app_state)
}
