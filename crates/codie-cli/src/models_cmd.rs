//! `codie models` command.

use anyhow::Result;
use clap::Args;
use codie_ai::builtin_models;

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

/// Lists the bundled model catalog.
pub(crate) fn run(args: ModelsArgs) -> Result<()> {
    let models = builtin_models()
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
        "{:<12} {:<30} {:>10} {:>10} {:>10} {:>10}",
        "provider", "model", "context", "output", "in $/M", "out $/M"
    );
    for model in models {
        println!(
            "{:<12} {:<30} {:>10} {:>10} {:>10.2} {:>10.2}",
            model.provider,
            model.id,
            model.context_window,
            model.max_output_tokens,
            model.cost.input,
            model.cost.output,
        );
    }
    Ok(())
}
