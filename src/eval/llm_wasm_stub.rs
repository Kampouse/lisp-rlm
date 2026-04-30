//! WASM stub for llm_provider — no actual LLM support

use std::sync::LazyLock;

pub trait LlmProvider: Send + Sync {
    fn box_clone(&self) -> Box<dyn LlmProvider>;
}

#[derive(Clone)]
pub struct GenericProvider;

impl LlmProvider for GenericProvider {
    fn box_clone(&self) -> Box<dyn LispProvider> { Box::new(GenericProvider) }
}

pub struct LlmResponse {
    pub text: String,
}

pub static SHARED_RUNTIME: LazyLock<()> = LazyLock::new(|| ());
