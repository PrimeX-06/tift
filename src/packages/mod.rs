//! Built-in package metadata, grouped by grammar and registered in one place.
//!
//! Source declarations and explicit configuration are applied by the callers.
//! Provider order here preserves precedence between overlapping definitions.

pub(crate) mod additional;
pub(crate) mod cjk;
pub(crate) mod classes;
pub(crate) mod classic;
pub(crate) mod code;
pub(crate) mod code_environments;
pub(crate) mod commands;
pub(crate) mod common_extensions;
pub(crate) mod common_more;
pub(crate) mod documents;
pub(crate) mod environments;
pub(crate) mod indexing;
pub(crate) mod keys;
pub(crate) mod lua;
pub(crate) mod math;
pub(crate) mod table_rules;
pub(crate) mod tables;
pub(crate) mod tikz_derived;
pub(crate) mod tikz_diagrams;
pub(crate) mod tikz_utilities;
pub(crate) mod visual_science;

use crate::config::CommandRule;
use std::collections::BTreeMap;

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    math::header_signature(name)
        .or_else(|| tables::header_signature(name))
        .or_else(|| classes::header_signature(name))
        .or_else(|| documents::header_signature(name))
        .or_else(|| additional::header_signature(name))
        .or_else(|| lua::header_signature(name))
        .or_else(|| code::header_signature(name))
        .or_else(|| code_environments::header_signature(name))
        .or_else(|| environments::header_signature(name))
        .or_else(|| visual_science::header_signature(name))
        .or_else(|| tikz_derived::header_signature(name))
        .or_else(|| common_extensions::header_signature(name))
        .or_else(|| common_more::header_signature(name))
        .or_else(|| classic::header_signature(name))
        .or_else(|| cjk::header_signature(name))
        .or_else(|| tikz_diagrams::header_signature(name))
}

pub(crate) fn scoped_header_signature(name: &str) -> Option<&'static str> {
    visual_science::header_signature(name)
        .or_else(|| tikz_diagrams::header_signature(name))
        .or_else(|| classic::header_signature(name))
        .or_else(|| cjk::header_signature(name))
}

pub(crate) fn scoped_invocation_signature(
    source: &str,
    cursor: usize,
    name: &str,
) -> Option<&'static str> {
    visual_science::invocation_signature(source, cursor, name)
        .or_else(|| tikz_diagrams::header_signature(name))
        .or_else(|| classic::header_signature(name))
        .or_else(|| cjk::header_signature(name))
}

pub(crate) fn scoped_environment_for_command(name: &str) -> Option<&str> {
    if let Some(environment) =
        cjk::environment_for_command(name).or_else(|| classic::environment_for_command(name))
    {
        return Some(environment);
    }
    if let Some(environment) = visual_science::environment_for_command(name) {
        Some(environment)
    } else if tikz_diagrams::header_signature(name).is_some() {
        Some(name)
    } else {
        name.strip_prefix("end")
            .filter(|name| tikz_diagrams::header_signature(name).is_some())
    }
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    math::command(name)
        .map(|signature| signature.shape)
        .or_else(|| documents::command_signature(name.trim_end_matches('*')))
        .or_else(|| commands::command_signature(source, name, cursor))
        .or_else(|| keys::command_signature(name))
        .or_else(|| visual_science::command_signature(source, name, cursor))
        .or_else(|| common_extensions::command_signature(source, name, cursor))
        .or_else(|| common_more::command_signature(source, name, cursor))
        .or_else(|| classic::command_signature(source, name, cursor))
        .or_else(|| cjk::command_signature(source, name, cursor))
        .or_else(|| indexing::command_signature(source, name, cursor))
        .or_else(|| tikz_utilities::command_signature(source, name, cursor))
        .or_else(|| tikz_derived::command_signature(name.trim_end_matches('*')))
}

pub(crate) fn has_scoped_command_data(name: &str) -> bool {
    commands::is_metadata_command(name)
        || keys::is_metadata_command(name)
        || visual_science::is_metadata_command(name)
        || common_more::is_metadata_command(name)
        || classic::is_metadata_command(name)
        || cjk::is_metadata_command(name)
        || indexing::is_metadata_command(name)
        || tikz_utilities::is_metadata_command(name)
        || tikz_diagrams::is_data_command(name)
        || matches!(name, "addplot" | "pgfplotstableread" | "texttiming")
}

/// Package-specific literal prefixes share one precedence order with the scanner.
pub(crate) fn command_data_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    commands::command_header_end(source, name, cursor)
        .or_else(|| keys::command_header_end(source, name, cursor))
        .or_else(|| visual_science::command_header_end(source, name, cursor))
        .or_else(|| common_more::command_header_end(source, name, cursor))
        .or_else(|| classic::command_header_end(source, name, cursor))
        .or_else(|| cjk::command_header_end(source, name, cursor))
        .or_else(|| indexing::command_header_end(source, name, cursor))
        .or_else(|| tikz_utilities::command_header_end(source, name, cursor))
}

pub(crate) fn column_spec_slot(name: &str) -> Option<usize> {
    tables::column_spec_slot(name)
        .or_else(|| environments::column_spec_slot(name))
        .or_else(|| visual_science::column_spec_slot(name))
}

pub(crate) fn alignment_environments() -> impl Iterator<Item = &'static str> {
    tables::ALIGNMENT_ENVIRONMENTS
        .iter()
        .chain(environments::ALIGNMENT_ENVIRONMENTS)
        .chain(additional::ALIGNMENT_ENVIRONMENTS)
        .chain(visual_science::ALIGNMENT_ENVIRONMENTS)
        .chain(tikz_derived::ALIGNMENT_ENVIRONMENTS)
        .chain(tikz_diagrams::ALIGNMENT_ENVIRONMENTS)
        .chain(math::ALIGNMENT_ENVIRONMENTS)
        .copied()
}

pub(crate) fn is_alignment_environment(name: &str) -> bool {
    tables::is_table(name)
        || environments::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || additional::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || visual_science::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || tikz_derived::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || tikz_diagrams::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || math::ALIGNMENT_ENVIRONMENTS.contains(&name)
}

pub(crate) fn default_list_environments() -> impl Iterator<Item = &'static str> {
    environments::ADDITIONAL_LIST_ENVIRONMENTS.iter().copied()
}

pub(crate) fn implicit_list_environments() -> impl Iterator<Item = &'static str> {
    classes::LIST_ENVIRONMENTS
        .iter()
        .chain(documents::LIST_ENVIRONMENTS)
        .chain(additional::LIST_ENVIRONMENTS)
        .chain(classic::LIST_ENVIRONMENTS)
        .copied()
}

pub(crate) fn verbatim_environments() -> impl Iterator<Item = &'static str> {
    environments::ADDITIONAL_VERBATIM_ENVIRONMENTS
        .iter()
        .chain(documents::VERBATIM_ENVIRONMENTS)
        .chain(lua::VERBATIM_ENVIRONMENTS)
        .chain(code_environments::VERBATIM_ENVIRONMENTS)
        .chain(code::VERBATIM_ENVIRONMENTS)
        .copied()
}

/// Extend one registry directly; providers avoid allocating intermediate maps.
fn extend_commands(commands: &mut BTreeMap<String, CommandRule>, specs: &[(&str, usize, bool)]) {
    commands.extend(specs.iter().map(|&(name, arguments, optional_arguments)| {
        (
            name.into(),
            CommandRule {
                arguments: Some(arguments),
                optional_arguments,
                ..Default::default()
            },
        )
    }));
}

fn add_extra_commands(commands: &mut BTreeMap<String, CommandRule>) {
    additional::add_command_rules(commands);
    lua::add_command_rules(commands);
    code::add_command_rules(commands);
    code_environments::add_command_rules(commands);
    visual_science::add_command_rules(commands);
    tikz_derived::add_command_rules(commands);
    tikz_utilities::add_command_rules(commands);
    common_extensions::add_command_rules(commands);
    common_more::add_command_rules(commands);
    classic::add_command_rules(commands);
    cjk::add_command_rules(commands);
    indexing::add_command_rules(commands);
    keys::add_command_rules(commands);
    tikz_diagrams::add_command_rules(commands);
}
