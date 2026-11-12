//! Generates and verifies the published JSON schemas.

use std::path::Path;

use anyhow::{Context, Result};

/// Writes or checks `schemas/*.schema.json` against the Rust protocol types.
pub(crate) fn run(root: &Path, args: &[String]) -> Result<bool> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => anyhow::bail!("usage: cargo xtask schemas [--check]"),
    };

    let directory = root.join("schemas");
    if !check {
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("cannot create {}", directory.display()))?;
    }

    let mut current = true;
    for (name, schema) in codie_protocol::schema_documents() {
        let path = directory.join(name);
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&schema).context("cannot encode schema")?
        );

        if check {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            if existing != text {
                current = false;
                eprintln!("outdated: {}", path.display());
            } else {
                println!("ok: {}", path.display());
            }
        } else {
            std::fs::write(&path, text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }

    Ok(current)
}
