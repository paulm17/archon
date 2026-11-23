use axum::{
    extract::{Json, State},
    response::sse::{Event, Sse},
};
use futures::Stream;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::brain::{
    reasoning::{self, ReasoningConfig},
    types::{AskRequest, StreamEvent},
};
use tokio::sync::mpsc;

pub async fn handle_ask(
    State(state): State<Arc<crate::brain::AppState>>,
    Json(request): Json<AskRequest>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    println!("Brain received query: {}", request.query);
    // Log additional details about request if needed
    // debug!("Request History: {:?}", request.history);

    let (tx, rx) = mpsc::channel(100);

    let state_clone = state.server.clone();
    let query = request.query.clone();
    let history = request.history.clone().unwrap_or_default();

    tokio::spawn(async move {
        // Use the pre-initialized LLM client from state
        let llm_client_arc = state.llm_client.clone();
        let llm_client = llm_client_arc.as_ref();

        let mut config = ReasoningConfig::default();
        config.context_window_size = state.config.brain.context_window_size;

        // Create references to clients
        let planning_client_arc = state.planning_client.clone();
        let planning_client_ref = planning_client_arc.as_deref();

        match reasoning::generate_reasoned_answer(
            &state_clone,
            &query,
            history,
            &llm_client,
            planning_client_ref,
            &config,
            Some(tx.clone()),
        )
        .await
        {
            Ok(_) => {
                // Done event is sent by stream_answer usually, but if not we can send it here.
                // reasoning::generate_reasoned_answer calls stream_answer which sends Done.
                // However if it returned early (non-streaming path?), we might need to send Done.
                // But we passed a sender, so it should have used the streaming path or emitted events.
                tracing::info!("Reasoning task completed successfully for query: {}", query);
            }
            Err(e) => {
                tracing::error!("Reasoning task failed for query '{}': {}", query, e);
                let _ = tx.send(Ok(StreamEvent::Error(e.to_string()))).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|res| match res {
        Ok(event) => Event::default()
            .json_data(event)
            .map_err(|e| axum::Error::new(e)),
        Err(e) => Err(axum::Error::new(e)),
    });

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}
