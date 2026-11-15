//! Literal grammar for fancyvrb, fvextra, listings and minted code utilities.

use std::collections::BTreeMap;

use crate::config::CommandRule;
use crate::package_environments::VerbatimDeclaration;

// fancyvrb.sty:1143--1207 and fvextra.sty:1638--1703.
pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[
    "BVerbatim*",
    "LVerbatim*",
    "SaveVerbatim",
    "SaveVerbatim*",
    "VerbatimOut",
    "VerbatimOut*",
    "VerbatimBuffer",
];

pub(crate) const INLINE_CODE_COMMANDS: &[&str] = &["Verb", "SaveVerb"];

/// Exact switch names assigned by listings.sty/lstmisc.sty through `\let`
/// or `\lstKV@SetIf`. Their current runtime truth value remains unknown.
pub(crate) fn is_condition(name: &str) -> bool {
    matches!(
        name,
        "lst@if"
            | "lst@ifLmode"
            | "lst@ifactivechars"
            | "lst@ifbox"
            | "lst@ifconsecutivenumbers"
            | "lst@ifdisplaystyle"
            | "lst@ifdraft"
            | "lst@ifdropinput"
            | "lst@ifec"
            | "lst@iffancyvrb"
            | "lst@ifflexible"
            | "lst@iffontadjust"
            | "lst@ifgobbledws"
            | "lst@ifkeepspaces"
            | "lst@ifletter"
            | "lst@ifmode"
            | "lst@ifnewline"
            | "lst@ifnolol"
            | "lst@ifnumberbychapter"
            | "lst@ifprint"
            | "lst@ifsavemem"
            | "lst@ifshowlines"
            | "lst@ifshowspaces"
            | "lst@ifshowtabs"
            | "lst@ifupquote"
            | "lst@ifwhitespace"
            | "lst@ifbreakatwhitespace"
            | "lst@ifbreakautoindent"
            | "lst@ifbreaklines"
            | "lst@ifbstring"
            | "lst@ifexcomments"
            | "lst@ifexstrings"
            | "lst@ifextags"
            | "lst@iffirstintag"
            | "lst@ifframeround"
            | "lst@ifindexproc"
            | "lst@ifkeywords"
            | "lst@ifmakekey"
            | "lst@ifmakemacrouse"
            | "lst@ifmarkfirstintag"
            | "lst@ifmathescape"
            | "lst@ifnotag"
            | "lst@ifnumberblanklines"
            | "lst@ifnumberfirstline"
            | "lst@ifpodcomment"
            | "lst@ifprintpod"
            | "lst@ifprocname"
            | "lst@ifresetmargins"
            | "lst@ifsensitive"
            | "lst@ifsensitivedefed"
            | "lst@ifshowstringspaces"
            | "lst@iftexcl"
            | "lst@iftexcsincludebs"
            | "lst@ifusekeysintag"
            | "lst@ifpreservenumber"
            | "lst@BDOifnewline"
    )
}

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "Verbatim" | "Verbatim*" | "BVerbatim" | "BVerbatim*" | "LVerbatim" | "LVerbatim*"
        | "VerbatimBuffer" => "o",
        "SaveVerbatim" | "SaveVerbatim*" | "VerbatimOut" | "VerbatimOut*" => "om",
        _ => return None,
    })
}

fn skip_space(source: &str, mut position: usize) -> usize {
    while source
        .as_bytes()
        .get(position)
        .is_some_and(u8::is_ascii_whitespace)
    {
        position += 1;
    }
    position
}

// FVExtraReadVArg gives braces their grouping catcodes and makes backslashes
// and percent signs ordinary. Their spelling must not affect brace counting.
fn raw_group_end(source: &str, start: usize) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&b'{') {
        return None;
    }
    let mut depth = 1usize;
    for (offset, byte) in source.as_bytes()[start + 1..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset + 2);
                }
            }
            _ => (),
        }
    }
    None
}

/// The caller handles explicit syntax rules and source redefinitions first.
/// FancyVerb's star and key list precede a delimiter; SaveVerb adds a name.
pub(crate) fn inline_code_end(source: &str, name: &str, position: usize) -> Option<usize> {
    let base = name.trim_end_matches('*');
    if !INLINE_CODE_COMMANDS.contains(&base) {
        return None;
    }
    let mut cursor = skip_space(source, position);
    if source.as_bytes().get(cursor) == Some(&b'*') {
        cursor = skip_space(source, cursor + 1);
    }
    if source.as_bytes().get(cursor) == Some(&b'[') {
        cursor = skip_space(
            source,
            crate::syntax::balanced_end(source, cursor, b'[', b']')?,
        );
    }
    if base == "SaveVerb" {
        cursor = skip_space(
            source,
            crate::syntax::balanced_end(source, cursor, b'{', b'}')?,
        );
    }
    if source.as_bytes().get(cursor) == Some(&b'{') {
        return raw_group_end(source, cursor);
    }
    let delimiter = source[cursor..].chars().next()?;
    if delimiter.is_whitespace() {
        return None;
    }
    let body = cursor + delimiter.len_utf8();
    let line_end = source[body..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| body + offset);
    source[body..line_end]
        .find(delimiter)
        .map(|offset| body + offset + delimiter.len_utf8())
}

/// All counts describe the fixed public arguments in the installed sources.
pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // fancyvrb.sty:55,84--107,767--778,1141--1190,1242--1248.
            ("fvset", 1, false),
            ("VerbatimInput", 1, true),
            ("BVerbatimInput", 1, true),
            ("LVerbatimInput", 1, true),
            ("UseVerb", 1, true),
            ("UseVerbatim", 1, true),
            ("BUseVerbatim", 1, true),
            ("LUseVerbatim", 1, true),
            ("DefineVerbatimEnvironment", 3, false),
            ("CustomVerbatimEnvironment", 3, false),
            ("RecustomVerbatimEnvironment", 3, false),
            ("CustomVerbatimCommand", 3, false),
            ("RecustomVerbatimCommand", 3, false),
            // fvextra.sty:1744,1766; both accept one optional key list, no body.
            ("VerbatimInsertBuffer", 0, true),
            ("VerbatimClearBuffer", 0, true),
            // listings.sty:402--403,294--319,2068--2078; lstmisc.sty:893--909.
            ("lstset", 1, false),
            ("lstinputlisting", 1, true),
            ("lstdefinestyle", 2, true),
            // minted.sty:1675--1689,1848--1853,1943--1960.
            ("inputminted", 2, true),
            ("setminted", 1, true),
            ("setmintedinline", 1, true),
            ("usemintedstyle", 1, true),
            ("newmintedfile", 2, true),
        ],
    );
}

fn argument<'a>(source: &'a str, cursor: &mut usize) -> Option<&'a str> {
    let start = crate::trivia::skip(source, *cursor);
    let end = crate::syntax::balanced_end(source, start, b'{', b'}')?;
    *cursor = end;
    Some(&source[start + 1..end - 1])
}

fn literal_name(value: &str) -> Option<&str> {
    let name = value.trim();
    (!name.is_empty()
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || "*@:-".contains(character)))
    .then_some(name)
}

/// Known FancyVerb bases generate literal raw environment names and stars.
/// The surrounding source scanner supplies scope and redefinition guards.
pub(crate) fn source_declaration(
    source: &str,
    command: &str,
    end: usize,
) -> Option<VerbatimDeclaration> {
    if !matches!(
        command,
        "DefineVerbatimEnvironment" | "CustomVerbatimEnvironment" | "RecustomVerbatimEnvironment"
    ) {
        return None;
    }
    let mut cursor = end;
    let name = literal_name(argument(source, &mut cursor)?)?;
    let base = literal_name(argument(source, &mut cursor)?)?;
    header_signature(base)?;
    argument(source, &mut cursor)?;
    Some(VerbatimDeclaration {
        names: vec![name.into(), format!("{name}*")],
        end: cursor,
        line_delimited: false,
    })
}
