//! Built-in providers and statically ordered capability dispatch.
use super::{
    InvocationContext, authoring, circuits_timing, cjk, classes, code, code_environments, commands,
    documents, environments, equation_floats, indexing, keys, links_color_columns,
    listing_wrappers, lua, math, multilingual, page_styles, references, scientific, tables,
    tikz_diagrams, tikz_utilities, typography, visual_languages,
};
use crate::config::CommandRule;
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    Foundation,
    Documents,
    Languages,
    Layout,
    References,
    Alignment,
    Graphics,
    Literal,
    Scientific,
}

fn math_command(_: &str, name: &str, _: usize) -> Option<&'static str> {
    math::command(name).map(|signature| signature.shape)
}
fn document_command(_: &str, name: &str, _: usize) -> Option<&'static str> {
    documents::command_signature(name.trim_end_matches('*'))
}
fn key_command(_: &str, name: &str, _: usize) -> Option<&'static str> {
    keys::command_signature(name)
}
fn circuit_command(_: &str, name: &str, _: usize) -> Option<&'static str> {
    circuits_timing::command_signature(name.trim_end_matches('*'))
}
fn document_data(source: &str, name: &str, cursor: usize) -> Option<Vec<Range<usize>>> {
    authoring::command_signature(source, name, cursor)?;
    Some(
        authoring::command_header_end(source, name, cursor)
            .map(|end| std::iter::once(cursor..end).collect())
            .unwrap_or_default(),
    )
}

// Each provider declares its capabilities once. The macros below expand
// them into direct calls in capability priority order. Keeping dispatch static
// lets the optimizer inline the short name tests on the per-token hot path.
// Equal priorities follow declaration order.
macro_rules! providers {
    ($consumer:ident, $($parameters:tt)*) => {
        $consumer! {
            ($($parameters)*);
            Foundation {
                commands: (2, commands::command_signature),
                data_ends: (0, commands::command_header_end),
                metadata: (0, commands::is_metadata_command),
                data: (0, commands::command_data_ranges),
            }
            Foundation {
                commands: (3, key_command),
                data_ends: (1, keys::command_header_end),
                metadata: (1, keys::is_metadata_command),
                adders: (12, keys::add_command_rules),
            }
            Foundation {
                headers: (8, environments::header_signature),
            }
            Documents {
                headers: (2, classes::header_signature),
            }
            Documents {
                headers: (3, documents::header_signature),
                commands: (1, document_command),
                data: (0, documents::filename_ranges),
                metadata: (0, documents::is_file_reader),
                context_command: (1, documents::context_command_signature),
            }
            Documents {
                headers: (17, authoring::header_signature),
                commands: (13, authoring::command_signature),
                data_ends: (9, authoring::command_header_end),
                metadata: (9, authoring::is_metadata_command),
                adders: (15, authoring::add_command_rules),
                data: (1, document_data),
            }
            Languages {
                headers: (17, multilingual::header_signature),
                commands: (13, multilingual::command_signature),
                data_ends: (9, multilingual::command_header_end),
                metadata: (9, multilingual::is_metadata_command),
                adders: (15, multilingual::add_command_rules),
                data: (1, multilingual::command_data_ranges),
                branches: (1, multilingual::command_branch_ranges),
            }
            Languages {
                headers: (14, cjk::header_signature),
                commands: (8, cjk::command_signature),
                data_ends: (5, cjk::command_header_end),
                metadata: (5, cjk::is_metadata_command),
                adders: (10, cjk::add_command_rules),
            }
            Layout {
                headers: (4, equation_floats::header_signature),
                commands: (2, equation_floats::command_signature),
                adders: (0, equation_floats::add_command_rules),
            }
            Layout {
                headers: (11, links_color_columns::header_signature),
                commands: (5, links_color_columns::command_signature),
                adders: (7, links_color_columns::add_command_rules),
                data: (4, links_color_columns::command_data_ranges),
                context_invocation: (1, links_color_columns::context_invocation_ranges),
            }
            Layout {
                headers: (12, page_styles::header_signature),
                commands: (6, page_styles::command_signature),
                data_ends: (3, page_styles::command_header_end),
                metadata: (3, page_styles::is_metadata_command),
                adders: (8, page_styles::add_command_rules),
            }
            Layout {
                headers: (13, typography::header_signature),
                commands: (7, typography::command_signature),
                data_ends: (4, typography::command_header_end),
                metadata: (4, typography::is_metadata_command),
                adders: (9, typography::add_command_rules),
            }
            References {
                headers: (16, references::header_signature),
                commands: (12, references::command_signature),
                data_ends: (8, references::command_header_end),
                metadata: (8, references::is_metadata_command),
                adders: (14, references::add_command_rules),
                data: (0, references::command_data_ranges),
                context_command: (1, references::context_command_signature),
                context_data: (1, references::context_command_data_ranges),
                context_ambiguous: (1, references::context_ambiguous),
                items: (0, references::is_package_item),
                branches: (1, references::command_branch_ranges),
            }
            References {
                commands: (9, indexing::command_signature),
                data_ends: (6, indexing::command_header_end),
                metadata: (6, indexing::is_metadata_command),
                adders: (11, indexing::add_command_rules),
            }
            Alignment {
                headers: (1, tables::header_signature),
            }
            Graphics {
                headers: (9, visual_languages::header_signature),
                commands: (4, visual_languages::command_signature),
                data_ends: (2, visual_languages::command_header_end),
                metadata: (2, visual_languages::is_metadata_command),
                adders: (4, visual_languages::add_command_rules),
            }
            Graphics {
                headers: (10, circuits_timing::header_signature),
                commands: (11, circuit_command),
                adders: (5, circuits_timing::add_command_rules),
            }
            Graphics {
                headers: (15, tikz_diagrams::header_signature),
                adders: (13, tikz_diagrams::add_command_rules),
            }
            Graphics {
                commands: (10, tikz_utilities::command_signature),
                data_ends: (7, tikz_utilities::command_header_end),
                metadata: (7, tikz_utilities::is_metadata_command),
                adders: (6, tikz_utilities::add_command_rules),
            }
            Literal {
                headers: (6, code::header_signature),
                adders: (2, code::add_command_rules),
                context_data: (2, code::context_command_data_ranges),
            }
            Literal {
                headers: (7, code_environments::header_signature),
                adders: (3, code_environments::add_command_rules),
            }
            Literal {
                headers: (5, lua::header_signature),
                adders: (1, lua::add_command_rules),
            }
            Literal {
                headers: (19, listing_wrappers::header_signature),
                context_header: (1, listing_wrappers::context_header_signature),
            }
            Scientific {
                headers: (0, math::header_signature),
                commands: (0, math_command),
            }
            Scientific {
                headers: (18, scientific::header_signature),
                commands: (14, scientific::command_signature),
                data_ends: (10, scientific::command_header_end),
                metadata: (10, scientific::is_metadata_command),
                adders: (16, scientific::add_command_rules),
                data: (2, scientific::command_data_ranges),
                context_command: (0, scientific::context_command_signature),
                context_data: (0, scientific::context_command_data_ranges),
                context_ambiguous: (0, scientific::context_ambiguous),
                context_math: (0, scientific::context_math_signature),
                context_invocation: (0, scientific::context_invocation_ranges),
                context_header: (0, scientific::context_header_signature),
            }
        }
    };
}

macro_rules! validate_field {
    (commands) => {
        1u16 << 0
    };
    (data_ends) => {
        1u16 << 1
    };
    (metadata) => {
        1u16 << 2
    };
    (adders) => {
        1u16 << 3
    };
    (headers) => {
        1u16 << 4
    };
    (data) => {
        1u16 << 5
    };
    (branches) => {
        1u16 << 6
    };
    (context_invocation) => {
        1u16 << 7
    };
    (context_command) => {
        1u16 << 8
    };
    (context_data) => {
        1u16 << 9
    };
    (context_ambiguous) => {
        1u16 << 10
    };
    (items) => {
        1u16 << 11
    };
    (context_math) => {
        1u16 << 12
    };
    (context_header) => {
        1u16 << 13
    };
}
macro_rules! validate_providers {
    ((); $($category:ident { $($field:ident: ($priority:literal, $callback:path),)* })*) => {
        $(
            const _: () = {
                let _ = Category::$category;
                let mut used = 0u16;
                $(
                    let capability = validate_field!($field);
                    assert!(used & capability == 0, "duplicate provider capability");
                    used |= capability;
                    assert!($priority < 32, "capability priority must be below 32");
                )*
                let _ = used;
            };
        )*
    };
}
providers!(validate_providers,);

macro_rules! invoke {
    (first, $rank:literal, ($($argument:expr),*), ($priority:literal, $callback:path)) => {
        if $rank == $priority && let Some(value) = $callback($($argument),*) {
            return Some(value);
        }
    };
    (any, $rank:literal, ($($argument:expr),*), ($priority:literal, $callback:path)) => {
        if $rank == $priority && $callback($($argument),*) {
            return true;
        }
    };
    (each, $rank:literal, ($($argument:expr),*), ($priority:literal, $callback:path)) => {
        if $rank == $priority {
            $callback($($argument),*);
        }
    };
}

macro_rules! select_capability {
    (commands, $mode:ident, $arguments:tt, $rank:literal;
        commands: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (data_ends, $mode:ident, $arguments:tt, $rank:literal;
        data_ends: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (metadata, $mode:ident, $arguments:tt, $rank:literal;
        metadata: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (adders, $mode:ident, $arguments:tt, $rank:literal;
        adders: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (headers, $mode:ident, $arguments:tt, $rank:literal;
        headers: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (data, $mode:ident, $arguments:tt, $rank:literal;
        data: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (branches, $mode:ident, $arguments:tt, $rank:literal;
        branches: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (context_invocation, $mode:ident, $arguments:tt, $rank:literal;
        context_invocation: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (context_command, $mode:ident, $arguments:tt, $rank:literal;
        context_command: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (context_data, $mode:ident, $arguments:tt, $rank:literal;
        context_data: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (context_ambiguous, $mode:ident, $arguments:tt, $rank:literal;
        context_ambiguous: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (items, $mode:ident, $arguments:tt, $rank:literal;
        items: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (context_math, $mode:ident, $arguments:tt, $rank:literal;
        context_math: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    (context_header, $mode:ident, $arguments:tt, $rank:literal;
        context_header: $value:tt, $($rest:tt)*) => {
        invoke!($mode, $rank, $arguments, $value);
    };
    ($field:ident, $mode:ident, $arguments:tt, $rank:literal;
        $other:ident: $value:tt, $($rest:tt)*) => {
        select_capability!($field, $mode, $arguments, $rank; $($rest)*);
    };
    ($field:ident, $mode:ident, $arguments:tt, $rank:literal;) => {};
}

macro_rules! ordered_capabilities {
    ($field:ident, $mode:ident, $arguments:tt; $providers:tt;) => {};
    ($field:ident, $mode:ident, $arguments:tt;
        [$({$($capabilities:tt)*})*]; $rank:literal, $($rest:literal,)*) => {
        $(select_capability!($field, $mode, $arguments, $rank; $($capabilities)*);)*
        ordered_capabilities!($field, $mode, $arguments;
            [$({$($capabilities)*})*]; $($rest,)*);
    };
}
macro_rules! dispatch {
    (($field:ident, $mode:ident, $arguments:tt);
        $($category:ident {$($capabilities:tt)*})*) => {
        ordered_capabilities!($field, $mode, $arguments; [$({$($capabilities)*})*];
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
            16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,);
    };
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    providers!(dispatch, commands, first, (source, name, cursor));
    None
}
pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    providers!(dispatch, headers, first, (name));
    None
}
pub(crate) fn command_data_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    providers!(dispatch, data_ends, first, (source, name, cursor));
    None
}
pub(crate) fn has_scoped_command_data(name: &str) -> bool {
    providers!(dispatch, metadata, any, (name));
    tikz_diagrams::is_data_command(name)
        || matches!(name, "addplot" | "pgfplotstableread" | "texttiming")
}
pub(crate) fn add_commands(commands: &mut BTreeMap<String, CommandRule>) {
    providers!(dispatch, adders, each, (commands));
}
pub(crate) fn command_signature_in_context(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    providers!(
        dispatch,
        context_command,
        first,
        (source, name, cursor, context)
    );
    command_signature(source, name, cursor)
}
pub(crate) fn command_data_ranges_in_context(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<Vec<Range<usize>>> {
    providers!(
        dispatch,
        context_data,
        first,
        (source, name, cursor, context)
    );
    providers!(dispatch, data, first, (source, name, cursor));
    None
}
pub(crate) fn context_math_signature(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<math::Signature> {
    providers!(dispatch, context_math, first, (name, context));
    None
}
pub(crate) fn context_ambiguous(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    providers!(dispatch, context_ambiguous, first, (name, context));
    None
}
pub(crate) fn context_invocation_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<Result<Vec<crate::syntax::xparse::ArgumentSlot>, Range<usize>>> {
    providers!(
        dispatch,
        context_invocation,
        first,
        (source, name, cursor, context)
    );
    None
}
pub(crate) fn context_header_signature(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    providers!(dispatch, context_header, first, (name, context));
    None
}
pub(crate) fn is_package_item(name: &str, environment: &str) -> bool {
    providers!(dispatch, items, any, (name, environment));
    false
}
pub(crate) fn command_branch_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    providers!(dispatch, branches, first, (source, name, cursor));
    None
}
