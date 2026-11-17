//! WithArrows alignments and its repeated leading option arguments.

use super::{ArgumentSlot, CommandSpec, InvocationContext, Range};

pub(super) const ALIGNMENT_ENVIRONMENTS: &[&str] =
    &["WithArrows", "DispWithArrows", "DispWithArrows*"];

specifications! {
    "WithArrowsOptions" => ("m", "U", "m"),
    "WithArrowsNewStyle" => ("mm", "UU", "mm"),
}

pub(super) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "WithArrows" => "!o",
        // The starred wrapper delegates its remaining arguments directly.
        "DispWithArrows" | "DispWithArrows*" => "!d<> !o",
        _ => return None,
    })
}

pub(super) fn environment_for_command(name: &str) -> Option<&str> {
    if header_signature(name).is_some() {
        Some(name)
    } else {
        name.strip_prefix("end")
            .filter(|name| header_signature(name).is_some())
    }
}

pub(super) fn is_context_metadata(name: &str) -> bool {
    name.trim_end_matches('*') == "Arrow"
}

pub(super) fn context_specification(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<CommandSpec> {
    (name.trim_end_matches('*') == "Arrow"
        && ALIGNMENT_ENVIRONMENTS.contains(&context.environment()?))
    .then_some(CommandSpec::new("o d<> m !o", "UUTU", "o d<>"))
}

pub(super) fn command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<Vec<Range<usize>>> {
    let specification = context_specification(name, context)?;
    Some(match invocation_ranges(source, name, cursor, context)? {
        Ok(arguments) => arguments
            .into_iter()
            .filter_map(|argument| {
                (specification.domains.as_bytes().get(argument.slot) == Some(&b'U'))
                    .then_some(argument.range)
            })
            .collect(),
        Err(range) => vec![range],
    })
}

/// Logical indices account for absent optional arguments in every consumer.
pub(super) fn invocation_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<Result<Vec<ArgumentSlot>, Range<usize>>> {
    if name != "Arrow" || !ALIGNMENT_ENVIRONMENTS.contains(&context.environment()?) {
        return None;
    }
    Some((|| {
        let option = crate::syntax::xparse::Signature::parse("o").expect("valid package grammar");
        let mut next = crate::trivia::skip(source, cursor);
        let mut ranges = Vec::new();
        while source.as_bytes().get(next) == Some(&b'[') {
            let mut arguments = option
                .slot_ranges(source, next..source.len())
                .ok_or(next..source.len())?;
            let argument = arguments
                .pop()
                .expect("opening optional delimiter was present");
            next = argument.range.end;
            ranges.push(argument);
        }
        for (index, shape) in [(1, "d<>"), (2, "m"), (3, "!o")] {
            let signature =
                crate::syntax::xparse::Signature::parse(shape).expect("valid package grammar");
            let start = if shape.starts_with('!') {
                next
            } else {
                crate::trivia::skip(source, next)
            };
            let arguments = signature
                .slot_ranges(source, next..source.len())
                .ok_or(start..source.len())?;
            for mut argument in arguments {
                next = argument.range.end;
                argument.slot = index;
                ranges.push(argument);
            }
        }
        Ok(ranges)
    })())
}
