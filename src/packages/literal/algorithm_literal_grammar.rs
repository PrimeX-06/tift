//! Raw pseudocode bodies from the `algorithms` and `algorithmicx` packages.

use crate::{CompiledConfig, FormatError, source_analysis::SourceAnalysis, syntax};
use std::{collections::BTreeMap, ops::Range};

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[
    // algorithms/algorithmic.sty:123 and algorithmicx/algorithmicx.sty:74
    // both define `algorithmic` with one optional line-number frequency.
    // algpseudocode.sty loads algorithmicx and supplies the pseudocode commands
    // used inside this environment (algpseudocode.sty:28--29, 51--81).
    // algorithms/algorithm.sty:80--96 declares the `algorithm` float.
    // That float and algorithm2e environments stay on their own structural
    // paths; algorithm2e formatting remains in code_layout.rs.
    "algorithmic",
];

pub(crate) fn indent_lines(
    source: &str,
    config: &CompiledConfig,
    protected: &[Range<usize>],
    analysis: &SourceAnalysis<'_>,
) -> Result<BTreeMap<usize, usize>, FormatError> {
    let mut result = BTreeMap::new();
    if config
        .config
        .syntax
        .environment_arguments
        .contains_key("algorithmic")
        || analysis.declared_environments()?.contains("algorithmic")
    {
        return Ok(result);
    }
    for block in syntax::verbatim_blocks_with_ranges(source, protected) {
        if block.name != "algorithmic" {
            continue;
        }
        let context = analysis.model().invocation_context_at(block.begin.start);
        if context.possibly_has_package("algorithmic")
            || context.possibly_has_package("algpascal")
            || context.possibly_has_package("algc")
        {
            continue;
        }
        let start =
            crate::layout::signature_argument_ranges(source, block.body.clone(), "o".bytes())
                .last()
                .map_or(block.body.start, |arg| arg.end);
        let body = &source[start..block.body.end];
        if let Some(lines) = body_lines(body, config, context.has_package("algpseudocode")) {
            result.extend(lines.into_iter().map(|(at, depth)| (start + at, depth)));
            result.insert(block.begin.start, 0);
            result.insert(block.end.start, 0);
        }
    }
    Ok(result)
}

fn body_lines(
    body: &str,
    config: &CompiledConfig,
    mut recognized: bool,
) -> Option<Vec<(usize, usize)>> {
    let mut protected =
        syntax::protected_ranges_with_syntax(body, &config.config, &config.syntax).ok()?;
    protected.extend(crate::code_layout::math_ranges(body, &protected));
    let protected = crate::code_layout::merged(protected);
    let mut ranges = protected.iter().peekable();
    let mut lines = Vec::new();
    let mut stack = Vec::new();
    let mut braces = 0usize;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let end = offset + line.len();
        let first = offset + line.len() - line.trim_start_matches([' ', '\t']).len();
        let mut depth = stack.len() + 1;
        let editable = offset != 0
            && braces == 0
            && !line.trim().is_empty()
            && !syntax::is_protected(first, &protected);
        let mut pos = first;
        while pos < end {
            while ranges.next_if(|r| r.end <= pos).is_some() {}
            if let Some(r) = ranges.peek().filter(|r| r.start <= pos) {
                pos = r.end.min(end);
                continue;
            }
            match body.as_bytes()[pos] {
                b'%' => break,
                b'{' => braces += 1,
                b'}' => braces = braces.checked_sub(1)?,
                b'\\' => {
                    let (name, next) = syntax::command_at(body, pos)?;
                    if braces == 0 {
                        let close = match name {
                            "If" => Some("EndIf"),
                            "For" | "ForAll" => Some("EndFor"),
                            "While" => Some("EndWhile"),
                            "Loop" => Some("EndLoop"),
                            "Repeat" => Some("Until"),
                            "Function" => Some("EndFunction"),
                            "Procedure" => Some("EndProcedure"),
                            _ => None,
                        };
                        if let Some(close) = close {
                            stack.push(close);
                            recognized = true;
                        } else if matches!(name, "Else" | "ElsIf") {
                            if stack.last() != Some(&"EndIf") {
                                return None;
                            }
                            if pos == first {
                                depth -= 1;
                            }
                        } else if matches!(
                            name,
                            "EndIf"
                                | "EndFor"
                                | "EndWhile"
                                | "EndLoop"
                                | "Until"
                                | "EndFunction"
                                | "EndProcedure"
                        ) {
                            if stack.pop() != Some(name) {
                                return None;
                            }
                            if pos == first {
                                depth -= 1;
                            }
                        } else if matches!(name, "algstore" | "algrestore" | "alglanguage") {
                            return None;
                        }
                    }
                    pos = next;
                    continue;
                }
                _ => {}
            }
            pos += body[pos..].chars().next()?.len_utf8();
        }
        if editable {
            lines.push((first, depth));
        }
        offset = end;
    }
    (recognized && stack.is_empty() && braces == 0).then_some(lines)
}
