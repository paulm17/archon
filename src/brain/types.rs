use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct AskRequest {
    pub query: String,
    pub conversation_id: Option<String>,
    pub history: Option<Vec<Message>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum StreamEvent {
    #[serde(rename = "thought")]
    Thought(Thought),
    #[serde(rename = "token")]
    Token(String),
    #[serde(rename = "source")]
    Source(Source),
    #[serde(rename = "error")]
    Error(String),
    #[serde(rename = "done")]
    Done,
}

#[derive(Debug, Serialize, Clone)]
pub struct Thought {
    pub title: String,
    pub content: String,
    pub state: ThoughtState,
}

#[derive(Debug, Serialize, Clone)]
pub enum ThoughtState {
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Serialize, Clone)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}
