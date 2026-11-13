//! `codie models` command.

use anyhow::Result;
use clap::Args;
use codie_ai::{
    OpenRouterCatalog, PROVIDER_OPENROUTER, builtin_models, merge_models, provider_base_url,
};

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

/// Lists the bundled catalog plus cached OpenRouter coding models.
pub(crate) fn run(args: ModelsArgs, config: &Config) -> Result<()> {
    let openrouter = args
        .provider
        .as_deref()
        .is_none_or(|provider| provider == PROVIDER_OPENROUTER);

    let mut catalog_stale = false;
    let mut models = builtin_models();
    if openrouter {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| provider_base_url(PROVIDER_OPENROUTER).to_owned());
        let catalog = OpenRouterCatalog::new(config.cache_dir.join("openrouter-models.json"))
            .with_endpoint(format!("{}/models", base_url.trim_end_matches('/')));
        catalog_stale = catalog.has_stale_cache();
        models = merge_models(models, catalog.cached_coding_models());
    }

    let models = models
        .into_iter()
        .filter(|model| {
            args.provider
                .as_ref()
                .is_none_or(|provider| model.provider == provider.as_str())
        })
        .collect::<Vec<_>>();

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

    if openrouter && catalog_stale {
        eprintln!(
            "hint: the OpenRouter catalog cache is stale; run `codie` once to refresh it in the background"
        );
    }
    Ok(())
}
