//! Built-in package metadata, grouped by grammar and registered in one place.
//!
//! Source declarations and explicit configuration are applied by the callers.
//! Provider order here preserves precedence between overlapping definitions.

#[path = "foundation/argument_command_tables.rs"]
pub(crate) mod argument_command_tables;
pub(crate) use argument_command_tables as argument_tables;
#[path = "document/authoring/authoring_command_registry.rs"]
pub(crate) mod authoring_command_registry;
pub(crate) use authoring_command_registry as authoring;
#[path = "graphics/circuit_timing_grammar.rs"]
pub(crate) mod circuit_timing_grammar;
pub(crate) use circuit_timing_grammar as circuits_timing;
#[path = "language/cjk_command_grammar.rs"]
pub(crate) mod cjk_command_grammar;
pub(crate) use cjk_command_grammar as cjk;
#[path = "document/document_class_grammar.rs"]
pub(crate) mod document_class_grammar;
pub(crate) use document_class_grammar as classes;
#[path = "literal/code_literal_grammar.rs"]
pub(crate) mod code_literal_grammar;
pub(crate) use code_literal_grammar as code;
#[path = "literal/algorithm_literal_grammar.rs"]
pub(crate) mod algorithm_literal_grammar;
pub(crate) use algorithm_literal_grammar as code_algorithms;
#[path = "literal/literal_environment_grammar.rs"]
pub(crate) mod literal_environment_grammar;
pub(crate) use literal_environment_grammar as code_environments;
#[path = "foundation/foundation_command_grammar.rs"]
pub(crate) mod foundation_command_grammar;
pub(crate) use foundation_command_grammar as commands;
#[path = "package_load_context.rs"]
pub(crate) mod package_load_context;
pub(crate) use package_load_context as context;
#[path = "document/document_command_grammar.rs"]
pub(crate) mod document_command_grammar;
pub(crate) use document_command_grammar as documents;
#[path = "foundation/foundation_environment_grammar.rs"]
pub(crate) mod foundation_environment_grammar;
pub(crate) use foundation_environment_grammar as environments;
#[path = "layout/equation_float_grammar.rs"]
pub(crate) mod equation_float_grammar;
pub(crate) use equation_float_grammar as equation_floats;
#[path = "reference/index_command_grammar.rs"]
pub(crate) mod index_command_grammar;
pub(crate) use index_command_grammar as indexing;
#[path = "foundation/key_value_command_grammar.rs"]
pub(crate) mod key_value_command_grammar;
pub(crate) use key_value_command_grammar as keys;
#[path = "layout/link_color_column_grammar.rs"]
pub(crate) mod link_color_column_grammar;
pub(crate) use link_color_column_grammar as links_color_columns;
#[path = "literal/listing_wrapper_grammar.rs"]
pub(crate) mod listing_wrapper_grammar;
pub(crate) use listing_wrapper_grammar as listing_wrappers;
#[path = "literal/lua_literal_grammar.rs"]
pub(crate) mod lua_literal_grammar;
pub(crate) use lua_literal_grammar as lua;
#[path = "scientific/math_command_grammar.rs"]
pub(crate) mod math_command_grammar;
pub(crate) use math_command_grammar as math;
#[path = "language/multilingual_command_grammar.rs"]
pub(crate) mod multilingual_command_grammar;
pub(crate) use multilingual_command_grammar as multilingual;
#[path = "layout/page_style_grammar.rs"]
pub(crate) mod page_style_grammar;
pub(crate) use page_style_grammar as page_styles;
#[path = "reference/reference_term_link_grammar.rs"]
pub(crate) mod reference_term_link_grammar;
pub(crate) use reference_term_link_grammar as references;
#[path = "scientific/scientific_package_registry.rs"]
pub(crate) mod scientific_package_registry;
pub(crate) use scientific_package_registry as scientific;
#[path = "alignment/horizontal_rule_grammar.rs"]
pub(crate) mod horizontal_rule_grammar;
pub(crate) use horizontal_rule_grammar as table_rules;
#[path = "alignment/table_environment_grammar.rs"]
pub(crate) mod table_environment_grammar;
pub(crate) use table_environment_grammar as tables;
#[path = "graphics/tikz_diagram_grammar.rs"]
pub(crate) mod tikz_diagram_grammar;
pub(crate) use tikz_diagram_grammar as tikz_diagrams;
#[path = "graphics/tikz_utility_grammar.rs"]
pub(crate) mod tikz_utility_grammar;
pub(crate) use tikz_utility_grammar as tikz_utilities;
#[path = "layout/typography_command_grammar.rs"]
pub(crate) mod typography_command_grammar;
pub(crate) use typography_command_grammar as typography;
#[path = "graphics/graphics_environment_grammar.rs"]
pub(crate) mod graphics_environment_grammar;
pub(crate) use graphics_environment_grammar as visual_languages;

use crate::config::CommandRule;
pub(crate) use context::{ContextIndex, InvocationContext};
use std::collections::BTreeMap;

#[path = "capability_dispatch.rs"]
pub(crate) mod capability_dispatch;
pub(crate) use capability_dispatch as registry;
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
