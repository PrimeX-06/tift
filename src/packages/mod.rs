//! Built-in package metadata, grouped by grammar and registered in one place.
//!
//! Source declarations and explicit configuration are applied by the callers.
//! Provider order here preserves precedence between overlapping definitions.

#[path = "foundation/argument_tables.rs"]
mod argument_tables;
#[path = "document/authoring/mod.rs"]
pub(crate) mod authoring;
#[path = "graphics/circuits_timing.rs"]
pub(crate) mod circuits_timing;
#[path = "language/cjk.rs"]
pub(crate) mod cjk;
#[path = "document/classes.rs"]
pub(crate) mod classes;
#[path = "literal/code.rs"]
pub(crate) mod code;
#[path = "literal/algorithms.rs"]
pub(crate) mod code_algorithms;
#[path = "literal/environments.rs"]
pub(crate) mod code_environments;
#[path = "foundation/commands.rs"]
pub(crate) mod commands;
pub(crate) mod context;
#[path = "document/documents.rs"]
pub(crate) mod documents;
#[path = "foundation/environments.rs"]
pub(crate) mod environments;
#[path = "layout/equation_floats.rs"]
pub(crate) mod equation_floats;
#[path = "reference/indexing.rs"]
pub(crate) mod indexing;
#[path = "foundation/keys.rs"]
pub(crate) mod keys;
#[path = "layout/links_color_columns.rs"]
pub(crate) mod links_color_columns;
#[path = "literal/listing_wrappers.rs"]
pub(crate) mod listing_wrappers;
#[path = "literal/lua.rs"]
pub(crate) mod lua;
#[path = "scientific/math.rs"]
pub(crate) mod math;
#[path = "language/multilingual.rs"]
pub(crate) mod multilingual;
#[path = "layout/page_styles.rs"]
pub(crate) mod page_styles;
#[path = "reference/terms_and_links.rs"]
pub(crate) mod references;
pub(crate) mod scientific;
#[path = "alignment/table_rules.rs"]
pub(crate) mod table_rules;
#[path = "alignment/tables.rs"]
pub(crate) mod tables;
#[path = "graphics/tikz_diagrams.rs"]
pub(crate) mod tikz_diagrams;
#[path = "graphics/tikz_utilities.rs"]
pub(crate) mod tikz_utilities;
#[path = "layout/typography.rs"]
pub(crate) mod typography;
#[path = "graphics/visual_languages.rs"]
pub(crate) mod visual_languages;

use crate::config::CommandRule;
pub(crate) use context::{ContextIndex, InvocationContext};
use std::collections::BTreeMap;

mod registry;
pub(crate) use registry::{
    command_branch_ranges, command_data_end, command_data_ranges_in_context, command_signature,
    command_signature_in_context, context_ambiguous, context_header_signature,
    context_invocation_ranges, context_math_signature, has_scoped_command_data, header_signature,
    is_package_item,
};

pub(crate) fn scoped_header_signature(name: &str) -> Option<&'static str> {
    visual_languages::header_signature(name)
        .or_else(|| tikz_diagrams::header_signature(name))
        .or_else(|| typography::header_signature(name))
        .or_else(|| cjk::header_signature(name))
        .or_else(|| multilingual::header_signature(name))
        .or_else(|| {
            authoring::header_signature(name)
                .or_else(|| references::header_signature(name))
                .or_else(|| scientific::header_signature(name))
        })
}

pub(crate) fn scoped_invocation_signature(
    source: &str,
    cursor: usize,
    name: &str,
) -> Option<&'static str> {
    matches!(name, "function" | "variable" | "codedescribe")
        .then_some("om")
        .or_else(|| visual_languages::invocation_signature(source, cursor, name))
        .or_else(|| tikz_diagrams::header_signature(name))
        .or_else(|| typography::header_signature(name))
        .or_else(|| cjk::header_signature(name))
        .or_else(|| multilingual::header_signature(name))
        .or_else(|| {
            authoring::header_signature(name)
                .or_else(|| references::header_signature(name))
                .or_else(|| scientific::header_signature(name))
        })
}

pub(crate) fn scoped_environment_for_command(name: &str) -> Option<&str> {
    if let Some(environment) = multilingual::environment_for_command(name)
        .or_else(|| references::environment_for_command(name))
        .or_else(|| scientific::environment_for_command(name))
    {
        return Some(environment);
    }
    if authoring::header_signature(name).is_some() {
        return Some(name);
    }
    if let Some(environment) = name
        .strip_prefix("end")
        .filter(|environment| authoring::header_signature(environment).is_some())
    {
        return Some(environment);
    }
    if let Some(environment) =
        cjk::environment_for_command(name).or_else(|| typography::environment_for_command(name))
    {
        return Some(environment);
    }
    if let Some(environment) = visual_languages::environment_for_command(name) {
        Some(environment)
    } else if tikz_diagrams::header_signature(name).is_some() {
        Some(name)
    } else {
        name.strip_prefix("end")
            .filter(|name| tikz_diagrams::header_signature(name).is_some())
    }
}

/// Package-specific literal prefixes share one precedence order with the scanner.
pub(crate) fn column_spec_slot(name: &str) -> Option<usize> {
    tables::column_spec_slot(name)
        .or_else(|| environments::column_spec_slot(name))
        .or_else(|| visual_languages::column_spec_slot(name))
}

pub(crate) fn alignment_environments() -> impl Iterator<Item = &'static str> {
    tables::ALIGNMENT_ENVIRONMENTS
        .iter()
        .chain(environments::ALIGNMENT_ENVIRONMENTS)
        .chain(equation_floats::ALIGNMENT_ENVIRONMENTS)
        .chain(visual_languages::ALIGNMENT_ENVIRONMENTS)
        .chain(circuits_timing::ALIGNMENT_ENVIRONMENTS)
        .chain(tikz_diagrams::ALIGNMENT_ENVIRONMENTS)
        .chain(math::ALIGNMENT_ENVIRONMENTS)
        .chain(scientific::ALIGNMENT_ENVIRONMENTS)
        .copied()
}

pub(crate) fn is_alignment_environment(name: &str) -> bool {
    tables::is_table(name)
        || environments::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || equation_floats::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || visual_languages::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || circuits_timing::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || tikz_diagrams::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || math::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || scientific::ALIGNMENT_ENVIRONMENTS.contains(&name)
}

pub(crate) fn default_list_environments() -> impl Iterator<Item = &'static str> {
    environments::ADDITIONAL_LIST_ENVIRONMENTS.iter().copied()
}

pub(crate) fn implicit_list_environments() -> impl Iterator<Item = &'static str> {
    classes::LIST_ENVIRONMENTS
        .iter()
        .chain(documents::LIST_ENVIRONMENTS)
        .chain(equation_floats::LIST_ENVIRONMENTS)
        .chain(typography::LIST_ENVIRONMENTS)
        .chain(references::LIST_ENVIRONMENTS)
        .copied()
}

pub(crate) fn verbatim_environments() -> impl Iterator<Item = &'static str> {
    environments::ADDITIONAL_VERBATIM_ENVIRONMENTS
        .iter()
        .chain(documents::VERBATIM_ENVIRONMENTS)
        .chain(lua::VERBATIM_ENVIRONMENTS)
        .chain(code_algorithms::VERBATIM_ENVIRONMENTS)
        .chain(code_environments::VERBATIM_ENVIRONMENTS)
        .chain(code::VERBATIM_ENVIRONMENTS)
        .chain(listing_wrappers::VERBATIM_ENVIRONMENTS)
        .chain(references::VERBATIM_ENVIRONMENTS)
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
    registry::add_commands(commands);
}
