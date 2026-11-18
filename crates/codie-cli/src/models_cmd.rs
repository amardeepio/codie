//! `codie models` command.

use anyhow::Result;
use clap::Args;
use codie_ai::provider::{self, CatalogKind};
use codie_ai::{OpenAiCatalog, OpenRouterCatalog, merge_models};

use crate::config::Config;

/// Model listing options.
#[derive(Debug, Args)]
pub(crate) struct ModelsArgs {
    /// Restrict to one provider.
    #[arg(long)]
    pub(crate) provider: Option<String>,
    /// Emit JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Lists the bundled catalog plus every cached remote provider catalog.
pub(crate) fn run(args: ModelsArgs, config: &Config) -> Result<()> {
    let selected = args
        .provider
        .as_deref()
        .and_then(provider::find)
        .into_iter()
        .collect::<Vec<_>>();
    let specs = if selected.is_empty() && args.provider.is_none() {
        provider::all().iter().collect::<Vec<_>>()
    } else {
        selected
    };

    let mut catalog_stale = false;
    let mut models = Vec::new();
    for spec in specs {
        let mut provider_models = (spec.models)();
        let endpoint_base = config
            .base_url
            .clone()
            .filter(|_| args.provider.is_some() || spec.catalog == CatalogKind::OpenRouter)
            .unwrap_or_else(|| spec.base_url.to_owned());
        let endpoint = format!("{}/models", endpoint_base.trim_end_matches('/'));

        match spec.catalog {
            CatalogKind::OpenRouter => {
                let catalog =
                    OpenRouterCatalog::new(config.cache_dir.join("openrouter-models.json"))
                        .with_endpoint(endpoint);
                catalog_stale |= catalog.has_stale_cache();
                provider_models = merge_models(provider_models, catalog.cached_coding_models());
            }
            CatalogKind::OpenAiModels => {
                let catalog = OpenAiCatalog::new(
                    *spec,
                    config.cache_dir.join(format!("{}-models.json", spec.id)),
                )
                .with_endpoint(endpoint);
                catalog_stale |= catalog.has_stale_cache();
                provider_models = merge_models(provider_models, catalog.cached_models());
            }
            CatalogKind::Bundled => {}
        }
        models.extend(provider_models);
    }

    if args.json {
        let values = models
            .iter()
            .map(|model| {
                serde_json::json!({
                    "id": model.id,
                    "name": model.name,
                    "provider": model.provider,
                    "api": model.api,
                    "context_window": model.context_window,
                    "max_output_tokens": model.max_output_tokens,
                    "reasoning": model.reasoning,
                    "cost": {
                        "input": model.cost.input,
                        "output": model.cost.output,
                        "cache_read": model.cost.cache_read,
                        "cache_write": model.cost.cache_write,
                    },
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    if models.is_empty() {
        println!("No models found for the selected provider.");
        return Ok(());
    }

    println!(
        "{:<12} {:<38} {:>10} {:>10} {:>10} {:>10}",
        "provider", "model", "context", "output", "in $/M", "out $/M"
    );
    for model in models {
        println!(
            "{:<12} {:<38} {:>10} {:>10} {:>10.2} {:>10.2}",
            model.provider,
            model.id,
            model.context_window,
            model.max_output_tokens,
            model.cost.input,
            model.cost.output,
        );
    }

    if catalog_stale {
        eprintln!(
            "hint: a remote model catalog cache is stale; run `codie` once to refresh it in the background"
        );
    }
    Ok(())
}
