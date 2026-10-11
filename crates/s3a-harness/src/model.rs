use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

impl CompletionRequest {
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            model: None,
            temperature: Some(0.1),
            max_tokens: Some(1024),
            stop: None,
        }
    }

    pub fn prompt(user_prompt: impl Into<String>) -> Self {
        Self::new(vec![ChatMessage::user(user_prompt)])
    }

    pub fn with_system(system_prompt: impl Into<String>, user_prompt: impl Into<String>) -> Self {
        Self::new(vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_prompt),
        ])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug)]
pub enum ModelError {
    HttpError(String),
    JsonError(String),
    Timeout(String),
    ModelNotFound(String),
    ConfigError(String),
    Other(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpError(e) => write!(f, "Model HTTP error: {}", e),
            Self::JsonError(e) => write!(f, "Model JSON error: {}", e),
            Self::Timeout(e) => write!(f, "Model timeout: {}", e),
            Self::ModelNotFound(e) => write!(f, "Model not found: {}", e),
            Self::ConfigError(e) => write!(f, "Model configuration error: {}", e),
            Self::Other(e) => write!(f, "Model error: {}", e),
        }
    }
}

impl std::error::Error for ModelError {}

/// Abstract Model Trait implemented by local small models, HTTP adapters, and test mocks.
pub trait Model: Send + Sync {
    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, ModelError>;
    fn model_id(&self) -> &str;
}

/// OpenAI-compatible HTTP adapter for Ollama, llama.cpp, vLLM, and hosted providers.
#[derive(Debug, Clone)]
pub struct OpenAiHttpAdapter {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model_id: String,
    pub timeout: Duration,
}

impl OpenAiHttpAdapter {
    pub fn new(base_url: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            model_id: model_id.into(),
            timeout: Duration::from_secs(60),
        }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Model for OpenAiHttpAdapter {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let target_model = req.model.as_deref().unwrap_or(&self.model_id);

        let body = serde_json::json!({
            "model": target_model,
            "messages": req.messages,
            "temperature": req.temperature.unwrap_or(0.1),
            "max_tokens": req.max_tokens.unwrap_or(1024),
        });
        let body_str =
            serde_json::to_string(&body).map_err(|e| ModelError::JsonError(e.to_string()))?;

        let mut request = ureq::post(&endpoint)
            .timeout(self.timeout)
            .set("Content-Type", "application/json");

        if let Some(ref key) = self.api_key {
            request = request.set("Authorization", &format!("Bearer {}", key));
        }

        let response = match request.send_string(&body_str) {
            Ok(resp) => resp,
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp
                    .into_string()
                    .unwrap_or_else(|_| "Unknown HTTP error body".to_string());
                return Err(ModelError::HttpError(format!(
                    "HTTP status {}: {}",
                    code, text
                )));
            }
            Err(ureq::Error::Transport(e)) => {
                return Err(ModelError::HttpError(format!("Transport error: {}", e)));
            }
        };

        let resp_text = response
            .into_string()
            .map_err(|e| ModelError::HttpError(format!("Failed to read response body: {}", e)))?;

        let json_resp: Value = serde_json::from_str(&resp_text)
            .map_err(|e| ModelError::JsonError(format!("Failed to parse response JSON: {}", e)))?;

        let content = json_resp
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        let prompt_tokens = json_resp
            .get("usage")
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0) as u32;

        let completion_tokens = json_resp
            .get("usage")
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0) as u32;

        let total_tokens = json_resp
            .get("usage")
            .and_then(|u| u.get("total_tokens"))
            .and_then(|n| n.as_u64())
            .unwrap_or((prompt_tokens + completion_tokens) as u64)
            as u32;

        Ok(CompletionResponse {
            content,
            model: target_model.to_string(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
        })
    }
}

/// Mock Model for deterministic and offline unit/integration testing.
#[derive(Clone)]
pub struct MockModel {
    pub model_id: String,
    responses: Arc<Mutex<Vec<CompletionResponse>>>,
    call_log: Arc<Mutex<Vec<CompletionRequest>>>,
    default_response: Arc<Mutex<String>>,
}

impl MockModel {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            responses: Arc::new(Mutex::new(Vec::new())),
            call_log: Arc::new(Mutex::new(Vec::new())),
            default_response: Arc::new(Mutex::new("MockModel: OK".to_string())),
        }
    }

    pub fn with_default_response(self, text: impl Into<String>) -> Self {
        *self.default_response.lock().unwrap() = text.into();
        self
    }

    pub fn queue_response(&self, content: impl Into<String>) {
        let content_str = content.into();
        self.responses.lock().unwrap().push(CompletionResponse {
            content: content_str,
            model: self.model_id.clone(),
            prompt_tokens: 15,
            completion_tokens: 25,
            total_tokens: 40,
        });
    }

    pub fn get_call_count(&self) -> usize {
        self.call_log.lock().unwrap().len()
    }

    pub fn get_last_request(&self) -> Option<CompletionRequest> {
        self.call_log.lock().unwrap().last().cloned()
    }
}

impl Model for MockModel {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, ModelError> {
        self.call_log.lock().unwrap().push(req.clone());

        let mut queue = self.responses.lock().unwrap();
        if !queue.is_empty() {
            Ok(queue.remove(0))
        } else {
            let default_text = self.default_response.lock().unwrap().clone();
            Ok(CompletionResponse {
                content: default_text,
                model: self.model_id.clone(),
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            })
        }
    }
}
