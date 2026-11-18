//! Ollama provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, Model};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "ollama",
    name: "Ollama",
    env_var: "OLLAMA_API_KEY",
    requires_key: false,
    base_url: "http://127.0.0.1:11434/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "qwen3-coder",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    Vec::new()
}
