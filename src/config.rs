use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
pub struct ArchonConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub llm: crate::config::LLMConfig,
    pub fast_llm: Option<crate::config::LLMConfig>, // Optional specific config for fast model
    #[serde(default)]
    pub brain: BrainConfig,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BrainConfig {
    /// How many verses before and after each retrieved result to include for narrative context.
    #[serde(default = "default_context_window_size")]
    pub context_window_size: usize,
    pub planning: Option<String>,
    pub thinking: Option<String>,
}

fn default_context_window_size() -> usize {
    2
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            context_window_size: default_context_window_size(),
            planning: None,
            thinking: None,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub storage_path: String,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct LLMConfig {
    pub base_url: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
}

fn default_llm_base_url() -> String {
    "http://localhost:9000/v1".to_string()
}

fn default_llm_model() -> String {
    "llama-3.2-3b-instruct".to_string()
}

pub fn default_embedding_model() -> String {
    "mixedbread-ai/mxbai-embed-large-v1".to_string()
}

impl ArchonConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: ArchonConfig = toml::from_str(&content)?;
        Ok(config)
    }
}

// Implement Default for LLMConfig for serde default
impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            base_url: default_llm_base_url(),
            model: default_llm_model(),
            api_key: None,
            embedding_model: default_embedding_model(),
        }
    }
}
