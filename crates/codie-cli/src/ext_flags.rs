//! Deferred extension flags.
//!
//! codie's own flags are parsed by clap. Unknown long flags are collected
//! before parsing and validated against extension declarations after the
//! extensions have performed their initialize handshake (`CLI-008`).

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;

use clap::{ArgAction, Command, CommandFactory};

use crate::Cli;

/// Long flags split out of the command line.
#[derive(Debug, Default)]
pub(crate) struct DeferredFlags {
    /// Values keyed by flag name. Boolean flags map to an empty string.
    pub(crate) values: BTreeMap<String, String>,
}

/// Removes unknown long flags from `args`, returning the clap-compatible
/// arguments and the deferred values.
pub(crate) fn split(args: Vec<OsString>) -> (Vec<OsString>, DeferredFlags) {
    let command = Cli::command();
    let known = known_flags(&command);
    let mut kept = Vec::new();
    let mut deferred = DeferredFlags::default();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            kept.extend(args[index..].iter().cloned());
            break;
        }
        let Some(text) = arg.to_str() else {
            kept.push(arg.clone());
            index += 1;
            continue;
        };
        let Some(rest) = text.strip_prefix("--") else {
            kept.push(arg.clone());
            index += 1;
            continue;
        };
        let (name, value) = rest
            .split_once('=')
            .map_or((rest, None), |(name, value)| (name, Some(value)));
        if name.is_empty() {
            kept.push(arg.clone());
            index += 1;
            continue;
        }
        if !known.contains_key(name) {
            deferred
                .values
                .insert(name.to_owned(), value.unwrap_or("").to_owned());
            index += 1;
            continue;
        }
        kept.push(arg.clone());
        index += 1;
        if value.is_none() && known.get(name) == Some(&true) && index < args.len() {
            kept.push(args[index].clone());
            index += 1;
        }
    }
    (kept, deferred)
}

fn known_flags(command: &Command) -> HashMap<String, bool> {
    let mut flags = HashMap::new();
    flags.insert("help".to_owned(), false);
    flags.insert("version".to_owned(), false);
    collect(command, &mut flags);
    flags
}

fn collect(command: &Command, flags: &mut HashMap<String, bool>) {
    for arg in command.get_arguments() {
        if let Some(long) = arg.get_long() {
            let takes_value =
                matches!(
                    arg.get_action(),
                    ArgAction::Set | ArgAction::Append | ArgAction::SetTrue | ArgAction::Count
                ) && !matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::Count);
            flags.insert(long.to_owned(), takes_value);
        }
    }
    for subcommand in command.get_subcommands() {
        collect(subcommand, flags);
    }
}
