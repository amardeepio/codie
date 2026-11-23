//! Aggregate child usage accounting.

use std::sync::Mutex;

use codie_core::{Cost, Usage};

use crate::config::SubagentConfig;

#[derive(Debug, Default)]
struct Spent {
    tokens: u64,
    nanos: u64,
}

/// Tracks the process-wide child budget.
#[derive(Debug)]
pub(crate) struct BudgetMeter {
    max_tokens: Option<u64>,
    max_nanos: Option<u64>,
    spent: Mutex<Spent>,
}

impl BudgetMeter {
    pub(crate) fn new(config: &SubagentConfig) -> Self {
        Self {
            max_tokens: config.max_total_tokens,
            max_nanos: config
                .max_total_cost_usd
                .map(Cost::nanos_from_usd)
                .filter(|nanos| *nanos > 0),
            spent: Mutex::new(Spent::default()),
        }
    }

    /// Returns a budget error when no further child may start.
    pub(crate) fn check_start(&self) -> Result<(), String> {
        let spent = self.spent.lock().map_err(|_| {
            "subagent budget is unavailable because its lock was poisoned".to_owned()
        })?;
        if let Some(max) = self.max_tokens
            && spent.tokens >= max
        {
            return Err(format!(
                "Subagent token budget exhausted ({}/{} tokens).",
                spent.tokens, max
            ));
        }
        if let Some(max) = self.max_nanos
            && spent.nanos >= max
        {
            return Err(format!(
                "Subagent cost budget exhausted (${:.4}/${:.4}).",
                nanos_to_usd(spent.nanos),
                nanos_to_usd(max)
            ));
        }
        Ok(())
    }

    /// Records one completed child's usage.
    pub(crate) fn record(&self, usage: &Usage) {
        if let Ok(mut spent) = self.spent.lock() {
            spent.tokens = spent.tokens.saturating_add(usage.total_tokens);
            spent.nanos = spent.nanos.saturating_add(usage.cost.total);
        }
    }
}

fn nanos_to_usd(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000_000.0
}
