//! Literal raw-body grammar from PythonTeX, scontents, filecontentsdef and spverbatim.

use crate::config::CommandRule;
use std::collections::BTreeMap;

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[
    // pythontex.sty:1026,1098,1121,1256,1351--1356. These are the default
    // py/sympy/pylab families, including their independently named consoles.
    "pycode",
    "pyblock",
    "pyverbatim",
    "pysub",
    "pyconcode",
    "pyconsub",
    "pyconverbatim",
    "pyconsole",
    "sympycode",
    "sympyblock",
    "sympyverbatim",
    "sympysub",
    "sympyconcode",
    "sympyconsub",
    "sympyconverbatim",
    "sympyconsole",
    "pylabcode",
    "pylabblock",
    "pylabverbatim",
    "pylabsub",
    "pylabconcode",
    "pylabconsub",
    "pylabconverbatim",
    "pylabconsole",
    "pythontexcustomcode",
    "pygments",
    // scontents.sty:172--177; scontents-code.tex:634--688 stores raw lines
    // and tracks nested occurrences of the same literal environment name.
    "scontents",
    "verbatimsc",
    // filecontentsdef.sty:140--166, including the documented starred aliases.
    "filecontentsdef",
    "filecontentsdef*",
    "filecontentsdefstarred",
    "filecontentsgdef",
    "filecontentsgdef*",
    "filecontentsgdefstarred",
    "filecontentsdefmacro",
    "filecontentsgdefmacro",
    "filecontentshere",
    "filecontentshere*",
    "filecontentsherestarred",
    // spverbatim.sty:40--45 uses the standard literal verbatim delimiter.
    "spverbatim",
    // lyluatex.sty:218--238,283--289,311--316 records LilyPond input
    // through a Lua input-buffer callback. Its backslashes are score syntax.
    "lilypond",
    "ly",
    "lysavefrag",
];

pub(crate) const INLINE_VERBATIM_COMMANDS: &[&str] = &["spverb"];

pub(crate) fn is_line_delimited_environment(name: &str) -> bool {
    VERBATIM_ENVIRONMENTS.contains(&name) && (name.starts_with("py") || name.starts_with("sympy"))
}

pub(crate) fn ignores_end_line_tail(name: &str) -> bool {
    // FCD@reserved@b warns about and discards the material following its
    // literal end marker, then dispatches the end at the active newline.
    name.starts_with("filecontents") && VERBATIM_ENVIRONMENTS.contains(&name)
}

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "pythontexcustomcode" | "pygments" => "om",
        "filecontentsdef"
        | "filecontentsdef*"
        | "filecontentsdefstarred"
        | "filecontentsgdef"
        | "filecontentsgdef*"
        | "filecontentsgdefstarred" => "mm",
        "filecontentsdefmacro"
        | "filecontentsgdefmacro"
        | "filecontentshere"
        | "filecontentshere*"
        | "filecontentsherestarred" => "m",
        "scontents" | "lilypond" | "ly" => "o",
        "lysavefrag" => "m",
        "verbatimsc" | "spverbatim" => "",
        _ if is_line_delimited_environment(name) => "oo",
        _ => return None,
    })
}

/// End offset of a nested scontents body, including the final literal end.
/// Backslashes and percent signs in the raw body have ordinary character
/// catcodes, so each literal same-name begin/end contributes to nesting.
pub(crate) fn nested_environment_end(source: &str, body_start: usize, name: &str) -> Option<usize> {
    if name != "scontents" {
        return None;
    }
    let mut start = body_start;
    while matches!(source.as_bytes().get(start), Some(b' ' | b'\t' | b'\r')) {
        start += 1;
    }
    if source.as_bytes().get(start) == Some(&b'[') {
        start = crate::syntax::balanced_end(source, start, b'[', b']')?;
    }
    // scontents checks for junk after begin/options and starts raw collection
    // at the next physical line (scontents-code.tex:589--609).
    let mut cursor = start + source.get(start..)?.find('\n')? + 1;
    let mut depth = 1usize;
    while let Some(offset) = source.get(cursor..)?.find('\\') {
        cursor += offset;
        if source[cursor..].starts_with("\\begin{scontents}") {
            depth += 1;
            cursor += "\\begin{scontents}".len();
        } else if source[cursor..].starts_with("\\end{scontents}") {
            depth -= 1;
            cursor += "\\end{scontents}".len();
            if depth == 0 {
                return Some(cursor);
            }
        } else {
            cursor += 1;
        }
    }
    None
}

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // PythonTeX public settings use one fixed value, optionally preceded
            // by a family/session selector (pythontex.sty:73--78,250--293,387).
            ("setpythontexautoprint", 1, false),
            ("setpythontexautostdout", 1, false),
            ("setpythontexcontext", 1, false),
            ("restartpythontexsession", 1, false),
            ("setpythontexoutputdir", 1, false),
            ("setpythontexworkingdir", 1, false),
            ("setpythontexfv", 1, true),
            ("setpythontexpyglexer", 1, true),
            ("setpythontexpygopt", 1, true),
            ("setpythontexprettyprinter", 1, true),
            ("makepythontexfamily", 1, true),
            ("inputpygments", 2, true),
            ("setpygmentsfv", 1, true),
            ("setpygmentspygopt", 1, true),
            ("setpygmentsprettyprinter", 1, false),
            // scontents-code.tex:937,1060,1241,1289,1421--1440.
            ("getstored", 1, true),
            ("foreachsc", 1, true),
            ("typestored", 1, true),
            ("meaningsc", 1, true),
            ("mergesc", 1, true),
            ("setupsc", 1, false),
            ("cleanseqsc", 1, false),
            ("newenvsc", 1, true),
            // filecontentsdef.sty:167,175,188 accepts a control-sequence token
            // or a braced macro name through FCD@get.
            ("filecontentsprint", 1, false),
            ("filecontentsprintviascan", 1, false),
            ("filecontentsexec", 1, false),
        ],
    );
}
