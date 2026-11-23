use crate::brain::error::BrainError;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use reqwest::Client;
use serde::{Deserialize, Serialize}; // Added Deserialize
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tracing::{debug, error, info};

pub type LlmOptions = std::collections::HashMap<String, serde_json::Value>;

fn scatter_embedding_results(
    input_len: usize,
    indexes: &[usize],
    embeddings: Vec<Vec<f32>>,
) -> Result<Vec<Vec<f32>>, BrainError> {
    if indexes.len() != embeddings.len() {
        return Err(BrainError::Embedding(format!(
            "Embedding model returned {} vectors for {} inputs",
            embeddings.len(),
            indexes.len()
        )));
    }
    let mut outputs = vec![Vec::new(); input_len];
    for (index, embedding) in indexes.iter().copied().zip(embeddings) {
        let Some(output) = outputs.get_mut(index) else {
            return Err(BrainError::Embedding(format!(
                "Embedding result index {} is outside input length {}",
                index, input_len
            )));
        };
        *output = embedding;
    }
    Ok(outputs)
}

#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    base_url: String,
    pub model: String,
    embedding_model_name: String,
    embedding_model: Arc<Mutex<Option<TextEmbedding>>>,
    loaded_model: Arc<Mutex<Option<String>>>,
    options: LlmOptions,
    request_gate: Arc<AsyncMutex<()>>,
}

impl LlmClient {
    pub fn new(
        base_url: String,
        model: String,
        embedding_model_name: String,
        embedding_model: Arc<Mutex<Option<TextEmbedding>>>,
        options: LlmOptions,
    ) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();

        Self {
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            base_url,
            model,
            embedding_model_name,
            embedding_model,
            loaded_model: Arc::new(Mutex::new(None)),
            options,
            request_gate: Arc::new(AsyncMutex::new(())),
        }
    }

    pub(crate) fn resolve_embedding_model(name: &str) -> Result<EmbeddingModel, BrainError> {
        match name {
            "" | "mixedbread-ai/mxbai-embed-large-v1" => Ok(EmbeddingModel::MxbaiEmbedLargeV1),
            "BAAI/bge-small-en-v1.5" | "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
            other => Err(BrainError::Embedding(format!(
                "Unsupported embedding model: {}",
                other
            ))),
        }
    }

    pub async fn preload_embedding_model(
        embedding_model: Arc<Mutex<Option<TextEmbedding>>>,
        embedding_model_name: String,
    ) -> Result<(), BrainError> {
        tokio::task::spawn_blocking(move || {
            let mut model = embedding_model
                .lock()
                .map_err(|e| BrainError::Embedding(e.to_string()))?;
            if model.is_none() {
                let resolved_model = Self::resolve_embedding_model(&embedding_model_name)?;
                tracing::info!(
                    "Initializing Brain Embedding Model ({})...",
                    embedding_model_name
                );
                *model = Some(
                    TextEmbedding::try_new(
                        InitOptions::new(resolved_model).with_show_download_progress(true),
                    )
                    .map_err(|e| BrainError::Embedding(e.to_string()))?,
                );
            }
            Ok(())
        })
        .await
        .map_err(|e| BrainError::Embedding(e.to_string()))?
    }

    /// Generate an embedding for the given text
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, BrainError> {
        let mut embeddings = self.embed_many(&[text.to_string()]).await?;
        Ok(embeddings.pop().unwrap_or_default())
    }

    /// Embed several texts while initializing/locking fastembed only once.
    /// Empty inputs retain the singleton API's empty-vector behavior and all
    /// outputs stay in exactly the input order.
    pub async fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, BrainError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let model = self.embedding_model.clone();
        let embedding_model_name = self.embedding_model_name.clone();
        let non_empty = texts
            .iter()
            .enumerate()
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|(index, text)| (index, text.clone()))
            .collect::<Vec<_>>();
        if non_empty.is_empty() {
            return Ok(vec![Vec::new(); texts.len()]);
        }
        let non_empty_len = non_empty.len();
        let non_empty_indexes = non_empty.iter().map(|(index, _)| *index).collect::<Vec<_>>();
        let non_empty_texts = non_empty
            .iter()
            .map(|(_, text)| text.clone())
            .collect::<Vec<_>>();

        Self::preload_embedding_model(model.clone(), embedding_model_name).await?;

        let started = Instant::now();
        let embeddings = tokio::task::spawn_blocking(move || {
            let mut model = model
                .lock()
                .map_err(|e| BrainError::Embedding(e.to_string()))?;
            model
                .as_mut()
                .expect("embedding model should be initialized")
                .embed(non_empty_texts, None)
                .map_err(|e| BrainError::Embedding(e.to_string()))
        })
        .await
        .map_err(|e| BrainError::Embedding(e.to_string()))??;

        debug!(
            input_count = texts.len(),
            non_empty_count = non_empty_len,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "embedding batch complete"
        );
        scatter_embedding_results(texts.len(), &non_empty_indexes, embeddings)
    }

    /// Load an MLX model via VardaDB's native endpoint
    pub async fn load(&self, model_path: &str) -> Result<(), BrainError> {
        let url = format!("{}/llm/load", self.base_url);
        info!("Loading MLX model from {} via {}", model_path, url);

        #[derive(Serialize)]
        struct LoadRequest {
            model_path: String,
        }

        let request = LoadRequest {
            model_path: model_path.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| BrainError::Llm(format!("Load request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Load request failed [{}]: {}", status, body);
            return Err(BrainError::Llm(format!("HTTP {}: {}", status, body)));
        }

        let mut loaded_model = self
            .loaded_model
            .lock()
            .map_err(|e| BrainError::Llm(e.to_string()))?;
        *loaded_model = Some(model_path.to_string());

        Ok(())
    }

    /// Unload the currently loaded MLX model
    pub async fn unload(&self) -> Result<(), BrainError> {
        let unload_guard = self.request_gate.lock().await;
        let url = format!("{}/llm/unload", self.base_url);
        info!("Unloading current MLX model via {}", url);

        let response = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| BrainError::Llm(format!("Unload request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Unload request failed [{}]: {}", status, body);
            return Err(BrainError::Llm(format!("HTTP {}: {}", status, body)));
        }

        let mut loaded_model = self
            .loaded_model
            .lock()
            .map_err(|e| BrainError::Llm(e.to_string()))?;
        *loaded_model = None;

        drop(unload_guard);
        Ok(())
    }

    async fn ensure_loaded(&self) -> Result<(), BrainError> {
        if self.model.is_empty() {
            return Ok(());
        }
        let already_loaded = self
            .loaded_model
            .lock()
            .map_err(|e| BrainError::Llm(e.to_string()))?
            .as_ref()
            .map(|loaded| loaded == &self.model)
            .unwrap_or(false);
        if already_loaded {
            return Ok(());
        }
        info!("Ensuring MLX model is loaded: {}", self.model);
        self.load(&self.model).await?;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        Ok(())
    }

    async fn post_json_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
        log_label: &str,
    ) -> Result<reqwest::Response, BrainError> {
        let send_once = || async {
            self.client.post(url).json(body).send().await.map_err(|e| {
                error!("{} request failed to send: {}", log_label, e);
                BrainError::Llm(format!("Request failed: {}", e))
            })
        };

        let response = send_once().await?;
        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        if status.as_u16() == 400
            && body_text.contains("model_not_loaded")
            && !self.model.is_empty()
        {
            info!(
                "{} request reported model_not_loaded; loading {} and retrying",
                log_label, self.model
            );
            self.load(&self.model).await?;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;

            let retry_response = send_once().await?;
            if retry_response.status().is_success() {
                return Ok(retry_response);
            }

            let retry_status = retry_response.status();
            let retry_body = retry_response.text().await.unwrap_or_default();
            error!(
                "{} retry failed [{}]: {}",
                log_label, retry_status, retry_body
            );
            return Err(BrainError::Llm(format!(
                "HTTP {}: {}",
                retry_status, retry_body
            )));
        }

        error!("{} request failed [{}]: {}", log_label, status, body_text);
        Err(BrainError::Llm(format!("HTTP {}: {}", status, body_text)))
    }

    /// Generate a non-streaming response
    pub async fn generate(&self, prompt: &str) -> Result<String, BrainError> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        info!("Generating response (prompt length: {})", prompt.len());
        debug!("Request URL: {} {}", "POST", url);

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct ChatRequest {
            model: String,
            messages: Vec<Message>,
            stream: bool,
            max_tokens: Option<usize>,
            #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
            #[serde(flatten)]
            options: LlmOptions,
        }

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            stream: false,
            max_tokens: Some(800), // Prevent infinite generation hangs
            options: self.options.clone(),
        };

        if !request.options.is_empty() {
            debug!("Request options: {:?}", request.options);
        }

        // Deep logging of request
        if let Ok(body_json) = serde_json::to_string(&request) {
            debug!("LLM Geneation Request Body: {}", body_json);
        }

        let request_guard = self.request_gate.lock().await;
        self.ensure_loaded().await?;
        let request_body = serde_json::to_value(&request)?;
        let start = std::time::Instant::now();
        let response = self
            .post_json_with_retry(&url, &request_body, "Generation")
            .await?;

        let duration = start.elapsed();
        debug!(
            "Response received in {:?} with status: {}",
            duration,
            response.status()
        );

        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: MessageContent,
        }

        #[derive(Deserialize)]
        struct MessageContent {
            content: String,
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| BrainError::Llm(format!("Failed to parse response: {}", e)))?;

        let content = chat_response
            .choices
            .first()
            .ok_or_else(|| BrainError::Llm("No choices returned".to_string()))?
            .message
            .content
            .clone();

        drop(request_guard);
        Ok(content)
    }

    /// Generate a non-streaming response with a max_tokens cap.
    /// Used for short tasks like query resolution where the model should produce a single line.
    pub async fn generate_short(
        &self,
        prompt: &str,
        max_tokens: usize,
        json_mode: bool,
    ) -> Result<String, BrainError> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        info!(
            "Generating short response (prompt length: {})",
            prompt.len()
        );
        debug!("Request URL: {} {}", "POST", url);

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct ChatRequest {
            model: String,
            messages: Vec<Message>,
            stream: bool,
            max_tokens: usize,
            #[serde(skip_serializing_if = "Option::is_none")]
            response_format: Option<serde_json::Value>,
            #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
            #[serde(flatten)]
            options: LlmOptions,
        }

        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            stream: false,
            max_tokens,
            response_format: if json_mode {
                Some(serde_json::json!({ "type": "json_object" }))
            } else {
                None
            },
            options: self.options.clone(),
        };

        if let Ok(body_json) = serde_json::to_string(&request) {
            debug!("LLM Short Request Body: {}", body_json);
        }

        let request_guard = self.request_gate.lock().await;
        self.ensure_loaded().await?;
        let request_body = serde_json::to_value(&request)?;
        let start = std::time::Instant::now();
        let response = self
            .post_json_with_retry(&url, &request_body, "Short generation")
            .await?;

        let duration = start.elapsed();
        debug!(
            "Response received in {:?} with status: {}",
            duration,
            response.status()
        );

        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: MessageContent,
        }

        #[derive(Deserialize)]
        struct MessageContent {
            content: String,
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| BrainError::Llm(format!("Failed to parse response: {}", e)))?;

        let content = chat_response
            .choices
            .first()
            .ok_or_else(|| BrainError::Llm("No choices returned".to_string()))?
            .message
            .content
            .clone();

        drop(request_guard);
        Ok(content)
    }

    /// HyDE-style completion that generates a synthetic passage for semantic retrieval.
    pub async fn complete_hyde(
        &self,
        question: &str,
        query_variant: &str,
    ) -> Result<String, BrainError> {
        let prompt = format!(
            "You are preparing a semantic retrieval seed using hypothetically-deduced evidence (HyDE).\n\
             Question: {question}\n\
             Candidate search phrasing: {query_variant}\n\n\
             Write a short 3-5 sentence passage in biblical style that would likely answer the question.\n\
             Make it vivid and specific, focusing on the most probable characters, events, or teachings.\n\
             Do not mention that this is hypothetical and do not cite verse numbers.\n\
             Return only the passage text."
        );

        let generated = self.generate(&prompt).await?;
        Ok(generated.trim().to_string())
    }

    /// Generate JSON with constrained decoding
    pub async fn generate_json(&self, prompt: &str) -> Result<String, BrainError> {
        // For now, rely on prompt engineering as full JSON mode might vary by provider
        let prompt_with_json = format!("{}\n\nRespond EXACTLY with valid JSON.", prompt);
        self.generate(&prompt_with_json).await
    }

    /// Generate a streaming response (returns a channel receiver)
    pub async fn generate_stream(
        &self,
        prompt: &str,
    ) -> Result<mpsc::Receiver<String>, BrainError> {
        self.generate_stream_with_system(None, prompt).await
    }

    /// Generate a streaming response with an optional system message
    pub async fn generate_stream_with_system(
        &self,
        system_prompt: Option<&str>,
        prompt: &str,
    ) -> Result<mpsc::Receiver<String>, BrainError> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        info!(
            "Starting streaming generation (prompt length: {})",
            prompt.len()
        );
        debug!("Request URL: {} {}", "POST", url);

        #[derive(Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(Serialize)]
        struct ChatStreamRequest {
            model: String,
            messages: Vec<Message>,
            stream: bool,
            max_tokens: Option<usize>,
            #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
            #[serde(flatten)]
            options: LlmOptions,
        }

        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(Message {
                role: "system".to_string(),
                content: sys.to_string(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        });

        let request = ChatStreamRequest {
            model: self.model.clone(),
            messages,
            stream: false,          // VardaDB doesn't stream yet, simulate below
            max_tokens: Some(2000), // Prevent infinite generation hangs on models like Qwen MLX
            options: self.options.clone(),
        };

        if !request.options.is_empty() {
            debug!("Request options: {:?}", request.options);
        }

        if let Ok(body) = serde_json::to_string(&request) {
            debug!("Stream Request Body: {}", body);
        }

        let request_guard = self.request_gate.lock().await;
        self.ensure_loaded().await?;
        let request_body = serde_json::to_value(&request)?;
        let start = std::time::Instant::now();
        let response = self
            .post_json_with_retry(&url, &request_body, "Streaming generation")
            .await?;

        let duration = start.elapsed();
        debug!(
            "Stream Response received in {:?} with status: {}",
            duration,
            response.status()
        );

        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: MessageContent,
        }

        #[derive(Deserialize)]
        struct MessageContent {
            content: String,
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| BrainError::Llm(format!("Failed to parse response: {}", e)))?;

        let content = chat_response
            .choices
            .first()
            .ok_or_else(|| BrainError::Llm("No choices returned".to_string()))?
            .message
            .content
            .clone();

        // Create a channel to stream tokens
        let (tx, rx) = mpsc::channel::<String>(100);

        // Spawn a task to send the full text
        tokio::spawn(async move {
            // Send the entire content at once since VardaDB's native mlx-lm engine
            // currently doesn't support streaming.
            if let Err(e) = tx.send(content).await {
                debug!("Receiver dropped, could not simulate stream: {}", e);
            }
        });

        drop(request_guard);
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::scatter_embedding_results;

    #[test]
    fn embedding_batch_scatter_preserves_input_order_and_empty_slots() {
        let output = scatter_embedding_results(
            4,
            &[0, 2],
            vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        )
        .unwrap();
        assert_eq!(output[0], vec![1.0, 0.0]);
        assert!(output[1].is_empty());
        assert_eq!(output[2], vec![0.0, 1.0]);
        assert!(output[3].is_empty());
    }

    #[test]
    fn embedding_batch_scatter_rejects_mismatched_results() {
        let error = scatter_embedding_results(2, &[1], vec![]).unwrap_err();
        assert!(error.to_string().contains("returned 0 vectors for 1 inputs"));
    }
}
