use crate::brain::llm::LlmClient;
use crate::config::ArchonConfig;
use fastembed::TextEmbedding;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct BrainRuntime {
    pub embedding_model: Arc<Mutex<Option<TextEmbedding>>>,
    pub llm_client: Arc<LlmClient>,
    pub planning_client: Option<Arc<LlmClient>>,
}

impl BrainRuntime {
    pub fn new(config: &ArchonConfig, embedding_model: Arc<Mutex<Option<TextEmbedding>>>) -> Self {
        let mut llm_options = std::collections::HashMap::new();
        llm_options.insert("temperature".to_string(), serde_json::json!(0.0));

        let thinking_model = config.brain.thinking.clone().unwrap_or_default();
        let llm_client = Arc::new(LlmClient::new(
            config.llm.base_url.clone(),
            thinking_model,
            config.llm.embedding_model.clone(),
            embedding_model.clone(),
            llm_options,
        ));

        let planning_client = config.brain.planning.as_ref().map(|planning_model| {
            Arc::new(LlmClient::new(
                config.llm.base_url.clone(),
                planning_model.clone(),
                config.llm.embedding_model.clone(),
                embedding_model.clone(),
                std::collections::HashMap::new(),
            ))
        });

        Self {
            embedding_model,
            llm_client,
            planning_client,
        }
    }
}
