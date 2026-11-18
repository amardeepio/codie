//! vLLM provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, Model};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "vllm",
    name: "vLLM",
    env_var: "VLLM_API_KEY",
    requires_key: false,
    base_url: "http://127.0.0.1:8000/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "Qwen/Qwen3-Coder-30B-A3B-Instruct",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    Vec::new()
}
