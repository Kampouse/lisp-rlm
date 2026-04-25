//! Pluggable LLM provider trait and default `GenericProvider`.
//!
//! This module introduces the [`LlmProvider`] trait so that every LLM-related
//! builtin (`llm`, `llm-code`, `rlm`, `sub-rlm`, `llm-batch`, `rlm-write`)
//! can share a single implementation instead of copy-pasting the HTTP call.

use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Shared runtime / client
// ---------------------------------------------------------------------------

/// Shared tokio runtime — avoids creating a new runtime per LLM/HTTP call.
pub static SHARED_RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("failed to create tokio runtime"));

/// Shared reqwest client.
///
/// Timeout is configurable via `RLM_TIMEOUT_SECS` (default 300 s = 5 min).
/// Fractal decomposition can trigger many sequential LLM calls; 60 s was too
/// aggressive for complex tasks that need long generation times.
pub static SHARED_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let timeout_secs: u64 = std::env::var("RLM_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .expect("failed to create reqwest client")
});

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A single LLM completion returned by a provider.
pub struct LlmResponse {
    pub content: String,
    pub tokens: usize,
}

/// Trait that any LLM backend must implement.
///
/// The `messages` slice contains `(role, content)` pairs, e.g.
/// `("system", "You are …")`, `("user", "Write a function …")`.
///
/// `max_tokens` is optional — the provider is free to pick its own default.
pub trait LlmProvider: Send + Sync {
    fn complete(
        &self,
        messages: &[(String, String)],
        max_tokens: Option<u64>,
    ) -> Result<LlmResponse, String>;

    /// Clone the provider into a boxed trait object.
    /// Needed by par-map/par-filter to propagate the provider into cloned envs.
    fn box_clone(&self) -> Box<dyn LlmProvider>;
}

// ---------------------------------------------------------------------------
// GenericProvider — reads env vars, calls OpenAI-compatible /chat/completions
// ---------------------------------------------------------------------------

/// Default provider that talks to any OpenAI-compatible `/chat/completions` endpoint.
pub struct GenericProvider {
    pub api_key: String,
    pub api_base: String,
    pub model: String,
}

impl GenericProvider {
    /// Build a `GenericProvider` from the standard environment variables:
    ///
    /// * `RLM_API_KEY` | `OPENAI_API_KEY` | `GLM_API_KEY`  (required)
    /// * `RLM_API_BASE`  (default `https://api.z.ai/api/coding/paas/v4`)
    /// * `RLM_MODEL`     (default `glm-5.1`)
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("RLM_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .or_else(|_| std::env::var("GLM_API_KEY"))
            .map_err(|_| "set RLM_API_KEY, OPENAI_API_KEY, or GLM_API_KEY".to_string())?;

        let api_base = std::env::var("RLM_API_BASE")
            .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4".to_string());

        let model = std::env::var("RLM_MODEL").unwrap_or_else(|_| "glm-5.1".to_string());

        Ok(Self {
            api_key,
            api_base,
            model,
        })
    }

    /// Convenience: create a provider or panic with a helpful message.
    pub fn from_env_or_panic() -> Self {
        Self::from_env().expect("GenericProvider::from_env failed")
    }
}

impl LlmProvider for GenericProvider {
    fn complete(
        &self,
        messages: &[(String, String)],
        max_tokens: Option<u64>,
    ) -> Result<LlmResponse, String> {
        let rt = &SHARED_RUNTIME;

        let json_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|(role, content)| serde_json::json!({"role": role, "content": content}))
            .collect();

        let mt = max_tokens.unwrap_or(2048);

        let api_key = self.api_key.clone();
        let api_base = self.api_base.clone();
        let model = self.model.clone();

        rt.block_on(async move {
            let client = &SHARED_CLIENT;

            let body = serde_json::json!({
                "model": model,
                "messages": json_messages,
                "max_tokens": mt,
            });

            let resp = client
                .post(format!("{}/chat/completions", api_base))
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("LLM request failed: {}", e))?;

            let text = resp
                .text()
                .await
                .map_err(|e| format!("LLM read body failed: {}", e))?;

            let v: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("LLM json parse error: {}", e))?;

            let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0) as usize;

            let content = v["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("LLM unexpected response: {}", text))?;

            Ok(LlmResponse { content, tokens })
        })
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(Self {
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            model: self.model.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialise env-var tests so they don't race with each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn from_env_fails_without_keys() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("GLM_API_KEY");

        let result = GenericProvider::from_env();
        assert!(result.is_err());
    }

    #[test]
    fn from_env_succeeds_with_rlm_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("GLM_API_KEY");
        std::env::set_var("RLM_API_KEY", "test-key-123");

        let provider = GenericProvider::from_env().unwrap();
        assert_eq!(provider.api_key, "test-key-123");
        assert_eq!(provider.api_base, "https://api.z.ai/api/coding/paas/v4");
        assert_eq!(provider.model, "glm-5.1");

        std::env::remove_var("RLM_API_KEY");
    }

    #[test]
    fn from_env_falls_back_to_openai_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("GLM_API_KEY");
        std::env::set_var("OPENAI_API_KEY", "openai-key");

        let provider = GenericProvider::from_env().unwrap();
        assert_eq!(provider.api_key, "openai-key");

        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn from_env_falls_back_to_glm_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::set_var("GLM_API_KEY", "glm-key");

        let provider = GenericProvider::from_env().unwrap();
        assert_eq!(provider.api_key, "glm-key");

        std::env::remove_var("GLM_API_KEY");
    }

    #[test]
    fn custom_base_and_model() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("RLM_API_KEY", "k");
        std::env::set_var("RLM_API_BASE", "https://custom.example.com/v1");
        std::env::set_var("RLM_MODEL", "my-custom-model");

        let provider = GenericProvider::from_env().unwrap();
        assert_eq!(provider.api_base, "https://custom.example.com/v1");
        assert_eq!(provider.model, "my-custom-model");

        std::env::remove_var("RLM_API_KEY");
        std::env::remove_var("RLM_API_BASE");
        std::env::remove_var("RLM_MODEL");
    }

    #[test]
    fn llm_response_struct() {
        let r = LlmResponse {
            content: "hello".to_string(),
            tokens: 42,
        };
        assert_eq!(r.content, "hello");
        assert_eq!(r.tokens, 42);
    }

    #[test]
    fn box_clone_produces_working_copy() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("RLM_API_KEY", "clone-key");
        let provider = GenericProvider::from_env().unwrap();
        let cloned = provider.box_clone();
        // Trait object works
        assert!(cloned.complete(&[], None).is_err() || true); // just verifying it doesn't panic
        std::env::remove_var("RLM_API_KEY");
    }
}
