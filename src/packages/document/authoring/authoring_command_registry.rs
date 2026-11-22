//! Collected bodies, theorem declarations and review notes.
//!
//! Metadata prefixes are delayed tokens or settings. Printable text arguments
//! and environment bodies remain visible to the ordinary source model.

use std::collections::BTreeMap;

use crate::{config::CommandRule, syntax::arguments::signature_end};

macro_rules! commands {
    ($($entries:tt)*) => {
        crate::packages::argument_tables::argument_commands! { $($entries)* }
    };
}

#[path = "review_annotation_grammar.rs"]
mod review_annotation_grammar;
use review_annotation_grammar as review;
#[path = "theorem_environment_grammar.rs"]
mod theorem_environment_grammar;
use theorem_environment_grammar as theorems;
#[path = "document_writing_grammar.rs"]
mod document_writing_grammar;
use document_writing_grammar as writing;

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    theorems::header_signature(name).or_else(|| review::header_signature(name))
}

fn specification(name: &str) -> Option<(&'static str, &'static str)> {
    theorems::specification(name)
        .or_else(|| review::specification(name))
        .or_else(|| writing::specification(name))
}

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    theorems::add_command_rules(commands);
    review::add_command_rules(commands);
    writing::add_command_rules(commands);
}

fn starred(source: &str, name: &str, cursor: usize) -> bool {
    name.ends_with('*') || source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'*')
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    let (shape, _) = specification(name)?;
    let base = name.trim_end_matches('*');
    let star = starred(source, name, cursor);
    let separated = star && !name.ends_with('*');
    if separated && let Some(shape) = writing::separated_star_shape(base) {
        return Some(shape);
    }
    if matches!(
        base,
        "fxnote" | "fxwarning" | "fxerror" | "fxfatal" | "fixme"
    ) && star
    {
        return Some(if separated { "momm" } else { "omm" });
    }
    Some(shape)
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    specification(name).is_some_and(|(_, prefix)| !prefix.is_empty())
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let (_, mut prefix) = specification(name)?;
    if prefix.is_empty() {
        return None;
    }
    if matches!(
        name.trim_end_matches('*'),
        "fxnote" | "fxwarning" | "fxerror" | "fxfatal" | "fixme"
    ) && !name.ends_with('*')
        && starred(source, name, cursor)
    {
        prefix = "mo";
    }
    let shape = command_signature(source, name, cursor)?;
    if let Some(extended) = writing::separated_star_shape(name)
        && shape == extended
    {
        prefix = extended;
    }
    signature_end(source, cursor, shape)?;
    signature_end(source, cursor, prefix).filter(|finish| *finish > cursor)
}
