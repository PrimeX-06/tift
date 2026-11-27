//! Literal headers of common table environments.
//!
//! Signatures describe arguments before the table body: `o` skips ordinary
//! whitespace before a bracket option, `O` requires token adjacency, and `m`
//! consumes one mandatory TeX argument. Environment names are exact.

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &[
    "tabular",
    "tabu",
    "longtabu",
    "tabular*",
    "array",
    "+array",
    "tabularx",
    "tabulary",
    "longtable",
    "xltabular",
    "xtabular",
    "xtabular*",
    "mpxtabular",
    "mpxtabular*",
    "supertabular",
    "supertabular*",
    "mpsupertabular",
    "mpsupertabular*",
    "tblr",
    "longtblr",
    "talltblr",
    "booktabs",
    "longtabs",
    "talltabs",
    "NiceTabular",
    "NiceTabularX",
    "NiceTabular*",
];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // array.sty: \@array[#1]#2; longtable.sty: \LT@array[#1]#2.
        // xtab/supertabular likewise read a placement option before columns.
        "tabular" | "tabu" | "longtabu" | "array" | "longtable" | "xtabular" | "mpxtabular"
        | "supertabular" | "mpsupertabular" => "om",
        // Width precedes the placement option and column preamble.
        "tabular*" | "tabularx" | "tabulary" | "xtabular*" | "mpxtabular*" | "supertabular*"
        | "mpsupertabular*" => "mom",
        // xltabular consumes placement, then forwards width/columns to tabularx.
        "xltabular" => "omm",
        // tabularray's NewTblrEnviron uses { O{} m +b }. The mandatory inner
        // options include colspec; brackets after them belong to the first cell.
        "tblr" | "longtblr" | "talltblr" | "booktabs" | "longtabs" | "talltabs" => "om",
        // tabularray's amsmath library uses the same inner-option header.
        "+array" => "om",
        // nicematrix's trailing !O option is tight; spaced brackets are data.
        "NiceTabular" => "omO",
        "NiceTabularX" | "NiceTabular*" => "momO",
        _ => return None,
    })
}

pub(crate) fn is_table(name: &str) -> bool {
    header_signature(name).is_some()
}

/// These environments use tabularray's ordered inner cell/row/column keys.
pub(crate) fn is_tabularray(name: &str) -> bool {
    matches!(
        name,
        "tblr"
            | "longtblr"
            | "talltblr"
            | "booktabs"
            | "longtabs"
            | "talltabs"
            | "+array"
            | "+matrix"
            | "+bmatrix"
            | "+Bmatrix"
            | "+pmatrix"
            | "+vmatrix"
            | "+Vmatrix"
            | "+cases"
    )
}

/// Tables whose surrounding paragraph may require adjacent box boundaries.
/// The mathematical array has its own math-context boundary handling.
pub(crate) fn is_box(name: &str) -> bool {
    is_table(name) && !matches!(name, "array" | "+array")
}

/// Zero-based position in the header signature of the column preamble.
/// For tabularray this is the entire inner-option argument containing colspec.
/// In these table signatures it is always the last mandatory argument.
pub(crate) fn column_spec_slot(name: &str) -> Option<usize> {
    header_signature(name)?.rfind('m')
}

/// tabu.sty:639--670 reads a TeX dimension after `to`/`spread`, then
/// placement and the braced preamble. Width expressions remain opaque;
/// expansion-dependent arguments or missing terminators have no safe extent.
pub(crate) fn tabu_column_spec(
    source: &str,
    body: std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    let skip = |cursor| crate::trivia::skip_until(source, cursor, body.end);
    let mut cursor = skip(body.start);
    let tail = &source[cursor..body.end];
    if tail.starts_with("to") || tail.starts_with("spread") {
        cursor += if tail.starts_with("to") { 2 } else { 6 };
        cursor = skip(cursor);
        cursor = tabu_dimension_end(source, cursor, body.end)?;
        cursor = skip(cursor);
    }
    if source.as_bytes().get(cursor) == Some(&b'[') {
        cursor = crate::syntax::balanced_end(source, cursor, b'[', b']')?;
        if cursor > body.end {
            return None;
        }
        cursor = skip(cursor);
    }
    let end = crate::syntax::balanced_end(source, cursor, b'{', b'}')?;
    (end <= body.end).then_some(cursor..end)
}

fn tabu_dimension_end(source: &str, start: usize, limit: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if let Some(("dimexpr", end)) = crate::syntax::command_at(source, start) {
        let mut cursor = end;
        while cursor < limit && cursor - start <= 1024 {
            cursor = crate::trivia::skip_until(source, cursor, limit);
            if let Some((command, end)) = crate::syntax::command_at(source, cursor) {
                if command == "relax" {
                    return Some(end);
                }
                if !command.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                    return None;
                }
                cursor = end;
            } else if bytes
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || b".+-*/()".contains(byte))
            {
                cursor += 1;
            } else {
                return None;
            }
        }
        return None;
    }
    let mut cursor = start;
    if bytes.get(cursor).is_some_and(|byte| b"+-".contains(byte)) {
        cursor += 1;
    }
    let number_start = cursor;
    let mut point = false;
    while cursor < limit {
        match bytes[cursor] {
            b'0'..=b'9' => cursor += 1,
            b'.' | b',' if !point => {
                point = true;
                cursor += 1;
            }
            _ => break,
        }
    }
    let number = source[number_start..cursor]
        .bytes()
        .any(|byte| byte.is_ascii_digit());
    cursor = crate::trivia::skip_until(source, cursor, limit);
    if let Some((command, end)) = crate::syntax::command_at(source, cursor) {
        return (matches!(
            command,
            "linewidth"
                | "textwidth"
                | "columnwidth"
                | "hsize"
                | "vsize"
                | "textheight"
                | "paperwidth"
                | "paperheight"
                | "columnsep"
                | "tabcolsep"
                | "arraycolsep"
                | "baselineskip"
                | "linegoal"
        ) && end <= limit)
            .then_some(end);
    }
    if !number {
        return None;
    }
    let unit_end = cursor.checked_add(2)?;
    if unit_end <= limit
        && matches!(
            &bytes[cursor..unit_end],
            b"pt" | b"pc" | b"in" | b"bp" | b"cm" | b"mm" | b"dd" | b"cc" | b"sp" | b"ex" | b"em"
        )
    {
        Some(unit_end)
    } else {
        None
    }
}

/// Direct children only: a marker in a grouped cell or a nested environment
/// belongs to that scope. Both alignment and indentation use the same boundary.
pub(crate) fn code_after_markers(
    blocks: &[crate::syntax::SharedBlock],
    config: &crate::Config,
    analysis: &crate::source_analysis::SourceAnalysis<'_>,
) -> Vec<(usize, usize)> {
    use crate::syntax::BlockKind;
    if config.syntax.commands.contains_key("CodeAfter")
        || analysis.declared_commands().contains("CodeAfter")
    {
        return Vec::new();
    }
    let declared = analysis.declared_environments();
    blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| {
            let parent = block.parent?;
            let env = &blocks[parent];
            (block.kind == BlockKind::Command
                && block.name == "CodeAfter"
                && env.kind == BlockKind::Environment
                && env.name.contains("Nice")
                && crate::packages::is_alignment_environment(&env.name)
                && !declared.contains(env.name.as_str())
                && !config
                    .syntax
                    .environment_arguments
                    .contains_key(env.name.as_str()))
            .then_some((i, parent))
        })
        .collect()
}
