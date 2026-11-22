//! Exact table rule command arguments, separate from following data cells.

use crate::syntax;
use crate::trivia::skip_until as trivia;

/// Return the end of one complete rule. Optional arguments belong only to
/// commands that actually read them; a following `{a}` remains a table cell.
pub(crate) fn rule_end(
    source: &str,
    start: usize,
    limit: usize,
    environment: &str,
) -> Option<usize> {
    let source = source.get(..limit)?;
    let (name, mut cursor) = syntax::command_at(source, start)?;
    let tabularray = matches!(
        environment,
        "tblr" | "longtblr" | "talltblr" | "booktabs" | "longtabs" | "talltabs" | "+array"
    );
    let signature = match name {
        "hline" if tabularray => "o",
        "cline" if tabularray => "om",
        "SetHline" if tabularray => "omm",
        "SetHlines" if tabularray => "omm",
        // The tabularray booktabs library has no parenthesized trim argument,
        // and provides its own extra-rule and row-space commands.
        "cmidrule" | "cmidrulemore" if tabularray => "om",
        "addrowspace" if tabularray => "o",
        "hline" | "morecmidrules" | "endfirsthead" | "endhead" | "endfoot" | "endlastfoot" => "",
        "cline" | "hhline" | "Xhline" | "noalign" => "m",
        // arydshln.sty: \\hdashline[gap] and \\cdashline{i-j}[gap].
        "hdashline" => "o",
        "cdashline" => "mo",
        "Xcline" => "mm",
        "toprule" | "midrule" | "bottomrule" | "addlinespace" => "o",
        "cmidrule" => "opm",
        "specialrule" => "mmm",
        _ => return None,
    };
    for slot in signature.bytes() {
        let next = trivia(source, cursor, limit);
        let (open, close) = match slot {
            b'o' => (b'[', b']'),
            b'p' => (b'(', b')'),
            b'm' => (b'{', b'}'),
            _ => unreachable!(),
        };
        if source.as_bytes().get(next) != Some(&open) {
            if slot == b'm' {
                return None;
            }
            continue;
        }
        cursor = syntax::balanced_end(source, next, open, close)?;
        if cursor > limit {
            return None;
        }
    }
    (cursor <= limit).then_some(cursor)
}
