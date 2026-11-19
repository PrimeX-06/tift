//! Literal grammar for source listings, display environments and inline code.

use std::collections::BTreeMap;

use crate::config::CommandRule;
use crate::package_environments::{VerbatimDeclaration, VerbatimTerminator};

// fancyvrb.sty:1143--1207 and fvextra.sty:1638--1703.
pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[
    "BVerbatim*",
    "LVerbatim*",
    "SaveVerbatim",
    "SaveVerbatim*",
    "VerbatimOut",
    "VerbatimOut*",
    "VerbatimBuffer",
    // fancyvrb-ex.sty:49--171 records these examples with VerbatimOut before
    // displaying and executing the stored source. Gobble and line numbers make
    // their original physical lines significant.
    "Example",
    "CenterExample",
    "SideBySideExample",
    "PCenterExample",
    "PSideBySideExample",
    // alltt.sty and beamerbaseverbatim.sty:56,124 preserve visual source
    // spacing while retaining normal TeX command execution.
    "alltt",
    "semiverbatim",
    // verbatimbox.sty:71--146,233--250.
    "origverbbox",
    "verbbox",
    "myverbbox",
    "verbnobox",
    // codehigh.sty:83--160, pygmentex.sty:226--270 and verbments.sty:81--130.
    "codehigh",
    "codehigh*",
    "demohigh",
    "demohigh*",
    "pygmented",
    "pyglist",
];

pub(crate) fn obeys_tex_command_boundaries(name: &str) -> bool {
    matches!(name, "alltt" | "semiverbatim")
}

pub(crate) fn is_line_delimited_environment(name: &str) -> bool {
    matches!(name, "pygmented" | "pyglist")
}

pub(crate) const INLINE_CODE_COMMANDS: &[&str] = &[
    "Verb",
    "SaveVerb",
    "pyginline",
    "qverb",
    "fverb",
    "collectverb",
    "Collectverb",
    "verbdef",
    "Verbdef",
];

/// Fixed catcode setters in expl3-code.tex:9200--9230. Their N operand is a
/// character token; expanding arbitrary token-list operands is left to TeX.
pub(crate) fn catcode_value(name: &str) -> Option<u8> {
    Some(match name {
        "char_set_catcode_escape:N" => 0,
        "char_set_catcode_group_begin:N" => 1,
        "char_set_catcode_group_end:N" => 2,
        "char_set_catcode_math_toggle:N" => 3,
        "char_set_catcode_alignment:N" => 4,
        "char_set_catcode_end_line:N" => 5,
        "char_set_catcode_parameter:N" => 6,
        "char_set_catcode_math_superscript:N" => 7,
        "char_set_catcode_math_subscript:N" => 8,
        "char_set_catcode_ignore:N" => 9,
        "char_set_catcode_space:N" => 10,
        "char_set_catcode_letter:N" => 11,
        "char_set_catcode_other:N" => 12,
        "char_set_catcode_active:N" => 13,
        "char_set_catcode_comment:N" => 14,
        "char_set_catcode_invalid:N" => 15,
        _ => return None,
    })
}

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
        | "VerbatimBuffer" | "Example" | "CenterExample" | "SideBySideExample" => "o",
        "SaveVerbatim" | "SaveVerbatim*" | "VerbatimOut" | "VerbatimOut*" => "om",
        // Pst@@Example takes one required coordinate pair and optionally a
        // second pair. xparse's delimited grammar expresses that distinction.
        "PCenterExample" | "PSideBySideExample" => "o r() d()",
        "myverbbox" => "om",
        "verbbox" | "origverbbox" | "verbnobox" | "codehigh" | "codehigh*" | "demohigh"
        | "demohigh*" | "pygmented" | "pyglist" => "o",
        "alltt" | "semiverbatim" => "",
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
pub(crate) fn raw_group_end(source: &str, start: usize) -> Option<usize> {
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
    if base == "pyginline" {
        let mut cursor = position;
        optional_argument(source, &mut cursor);
        cursor = crate::trivia::skip(source, cursor);
        // The first mandatory argument supplies the opener itself, either
        // as a character token (|code|) or a braced token ({|}code|).
        let delimiter = if source.as_bytes().get(cursor) == Some(&b'{') {
            let value = argument(source, &mut cursor)?;
            let mut chars = value.chars();
            let delimiter = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            delimiter
        } else {
            let delimiter = source.get(cursor..)?.chars().next()?;
            cursor += delimiter.len_utf8();
            delimiter
        };
        return delimited_end(source, cursor, delimiter);
    }
    if matches!(base, "qverb" | "fverb") {
        return generated_inline_end(source, GeneratedInlineKind::Delimited, position);
    }
    if matches!(base, "collectverb" | "Collectverb" | "verbdef" | "Verbdef") {
        let mut cursor = crate::trivia::skip(source, position);
        if source.as_bytes().get(cursor) == Some(&b'*') {
            cursor = crate::trivia::skip(source, cursor + 1);
        }
        // Collectverb takes a TeX token/group callback; verbdef takes one
        // control sequence. Then braces become a pair of literal delimiters.
        if source.as_bytes().get(cursor) == Some(&b'{') {
            argument(source, &mut cursor)?;
        } else if let Some((name, end)) = crate::syntax::command_at(source, cursor) {
            cursor = end - usize::from(name.len() > 1 && name.ends_with('*'));
        } else {
            return None;
        }
        let delimiter = source.get(cursor..)?.chars().next()?;
        let close = if delimiter == '{' { '}' } else { delimiter };
        return delimited_end(source, cursor + delimiter.len_utf8(), close);
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
    delimited_end(source, cursor + delimiter.len_utf8(), delimiter)
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
            ("CodeHigh", 1, true),
            ("NewCodeHighEnv", 2, false),
            ("NewCodeHighInput", 2, false),
            ("dochighinput", 1, true),
            ("inputpygmented", 1, true),
            ("setpygmented", 1, false),
            ("plset", 1, false),
            ("verbfilebox", 1, true),
            ("verbfilenobox", 1, true),
            ("theverbbox", 0, true),
            ("listofpyglistings", 0, false),
            ("listofpyglistingsname", 1, false),
        ],
    );
}

/// verbatimcopy makes backslashes, comment characters, and filename spaces
/// ordinary under its default reader. A closed brace group also works in
/// compatibility mode, which uses the original TeX argument parser.
pub(crate) fn verbatim_copy_end(source: &str, name: &str, mut cursor: usize) -> Option<usize> {
    let count = match name {
        "VerbatimCopy" => 2,
        "setOutputDir" => 1,
        _ => return None,
    };
    for _ in 0..count {
        cursor = skip_space(source, cursor);
        let delimiter = source.get(cursor..)?.chars().next()?;
        cursor = if delimiter == '{' {
            raw_group_end(source, cursor)?
        } else {
            let body = cursor + delimiter.len_utf8();
            body + source[body..].find(delimiter)? + delimiter.len_utf8()
        };
    }
    Some(cursor)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GeneratedInlineKind {
    Delimited,
    FancyVerb,
    Minted,
    PythonTex,
}

impl GeneratedInlineKind {
    pub(crate) fn accepts_star_delimiter(self) -> bool {
        matches!(self, Self::Minted | Self::PythonTex)
    }
}

pub(crate) fn is_inline_factory(name: &str) -> bool {
    matches!(
        name,
        "newverbcommand"
            | "renewverbcommand"
            | "provideverbcommand"
            | "makepythontexfamily"
            | "newmint"
            | "newmintinline"
    )
}

pub(crate) fn pythontex_inline_end(source: &str, name: &str, end: usize) -> Option<usize> {
    match name {
        // PythonTeX: default families, including the restricted console forms.
        "py" | "pyb" | "pyc" | "pys" | "pyv" | "pycon" | "pyconc" | "pyconv" | "sympy"
        | "sympyb" | "sympyc" | "sympys" | "sympyv" | "sympycon" | "sympyconc" | "sympyconv"
        | "pylab" | "pylabb" | "pylabc" | "pylabs" | "pylabv" | "pylabcon" | "pylabconc"
        | "pylabconv" => generated_inline_end(source, GeneratedInlineKind::PythonTex, end),
        // Both call the inline reader with the session already supplied.
        "pygment" | "pythontexcustomc" => {
            let mut cursor = end;
            if name == "pythontexcustomc" {
                optional_argument(source, &mut cursor);
            }
            argument(source, &mut cursor)?;
            braced_or_delimited_code_end(source, cursor)
        }
        _ => None,
    }
}

pub(crate) fn is_pythontex_inline(name: &str) -> bool {
    matches!(
        name,
        "py" | "pyb"
            | "pyc"
            | "pys"
            | "pyv"
            | "pycon"
            | "pyconc"
            | "pyconv"
            | "sympy"
            | "sympyb"
            | "sympyc"
            | "sympys"
            | "sympyv"
            | "sympycon"
            | "sympyconc"
            | "sympyconv"
            | "pylab"
            | "pylabb"
            | "pylabc"
            | "pylabs"
            | "pylabv"
            | "pylabcon"
            | "pylabconc"
            | "pylabconv"
            | "pygment"
            | "pythontexcustomc"
    )
}

/// Parse the small set of source-level factories that create known inline
/// verbatim commands. Unknown reader macros stay outside the inferred set.
pub(crate) fn source_inline_declaration(
    source: &str,
    command: &str,
    end: usize,
) -> Option<(Vec<(String, GeneratedInlineKind)>, usize)> {
    let mut cursor = end;
    let declarations = match command {
        "newverbcommand" | "renewverbcommand" | "provideverbcommand" => {
            let target = command_argument_name(source, &mut cursor)?;
            let reader = optional_argument(source, &mut cursor)
                .map(|range| source[range].trim())
                .unwrap_or("\\verb");
            let kind = match reader {
                "\\verb" => GeneratedInlineKind::Delimited,
                "\\Verb" => GeneratedInlineKind::FancyVerb,
                _ => return None,
            };
            argument(source, &mut cursor)?;
            argument(source, &mut cursor)?;
            vec![(target, kind)]
        }
        "makepythontexfamily" => {
            let (family, console) = pythontex_family(source, &mut cursor)?;
            (if console {
                &["", "c", "s", "v"][..]
            } else {
                &["", "c", "s", "v", "b"]
            })
            .iter()
            .map(|suffix| (format!("{family}{suffix}"), GeneratedInlineKind::PythonTex))
            .collect()
        }
        "newmint" | "newmintinline" => {
            let explicit = optional_argument(source, &mut cursor)
                .map(|range| source[range].trim().trim_start_matches('\\').to_owned());
            let language = argument(source, &mut cursor)?.trim();
            argument(source, &mut cursor)?;
            let target = explicit.filter(|name| !name.is_empty()).unwrap_or_else(|| {
                if command == "newmint" {
                    language.to_owned()
                } else {
                    format!("{language}inline")
                }
            });
            let target = literal_control_word(&target)?.to_owned();
            vec![(target, GeneratedInlineKind::Minted)]
        }
        _ => return None,
    };
    Some((declarations, cursor))
}

fn command_argument_name(source: &str, cursor: &mut usize) -> Option<String> {
    let start = crate::trivia::skip(source, *cursor);
    if source.as_bytes().get(start) == Some(&b'{') {
        let end = crate::syntax::balanced_end(source, start, b'{', b'}')?;
        let inner = source[start + 1..end - 1].trim();
        let name = inner.strip_prefix('\\')?;
        literal_control_word(name)?;
        *cursor = end;
        Some(name.into())
    } else {
        let (name, end) = crate::syntax::command_at(source, start)?;
        literal_control_word(name)?;
        *cursor = end;
        Some(name.into())
    }
}

fn optional_argument(source: &str, cursor: &mut usize) -> Option<std::ops::Range<usize>> {
    let start = crate::trivia::skip(source, *cursor);
    if source.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let end = crate::syntax::balanced_end(source, start, b'[', b']')?;
    *cursor = end;
    Some(start + 1..end - 1)
}

fn literal_control_word(value: &str) -> Option<&str> {
    let name = value.trim();
    (!name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'@'))
    .then_some(name)
}

fn pythontex_family<'a>(source: &'a str, cursor: &mut usize) -> Option<(&'a str, bool)> {
    let options = optional_argument(source, cursor);
    let family = literal_control_word(argument(source, cursor)?)?;
    let console = options.is_some_and(|range| {
        source[range]
            .split(',')
            .rev()
            .filter_map(|option| option.split_once('='))
            .find(|(key, _)| key.trim() == "console")
            .is_some_and(|(_, value)| value.trim() == "true")
    });
    Some((family, console))
}

/// End of one generated command body. PythonTeX takes an optional session
/// bracket before a braced or delimiter-terminated code argument.
pub(crate) fn generated_inline_end(
    source: &str,
    kind: GeneratedInlineKind,
    position: usize,
) -> Option<usize> {
    if kind == GeneratedInlineKind::FancyVerb {
        return inline_code_end(source, "Verb", position);
    }
    if kind == GeneratedInlineKind::Minted {
        let mut cursor = position;
        optional_argument(source, &mut cursor);
        return braced_or_delimited_code_end(source, skip_space(source, cursor));
    }
    let mut cursor = skip_space(source, position);
    if kind == GeneratedInlineKind::PythonTex && source.as_bytes().get(cursor) == Some(&b'[') {
        // [ is read with backslash and percent as ordinary characters.
        cursor += source.get(cursor..)?.find(']')? + 1;
    }
    if kind == GeneratedInlineKind::PythonTex {
        return braced_or_delimited_code_end(source, cursor);
    }
    if source.as_bytes().get(cursor) == Some(&b'*') {
        cursor = skip_space(source, cursor + 1);
    }
    let delimiter = source[cursor..].chars().next()?;
    delimited_end(source, cursor + delimiter.len_utf8(), delimiter)
}

fn braced_or_delimited_code_end(source: &str, cursor: usize) -> Option<usize> {
    if source.as_bytes().get(cursor) == Some(&b'{') {
        return raw_group_end(source, cursor);
    }
    let delimiter = source.get(cursor..)?.chars().next()?;
    delimited_end(source, cursor + delimiter.len_utf8(), delimiter)
}

fn delimited_end(source: &str, body: usize, delimiter: char) -> Option<usize> {
    if delimiter.is_whitespace() {
        return None;
    }
    let line_end = source[body..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| body + offset);
    source[body..line_end]
        .find(delimiter)
        .map(|offset| body + offset + delimiter.len_utf8())
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
    let mut cursor = end;
    match command {
        "DefineVerbatimEnvironment"
        | "CustomVerbatimEnvironment"
        | "RecustomVerbatimEnvironment" => {
            let name = literal_name(argument(source, &mut cursor)?)?;
            let base = literal_name(argument(source, &mut cursor)?)?;
            header_signature(base)?;
            argument(source, &mut cursor)?;
            Some(VerbatimDeclaration {
                names: vec![name.into(), format!("{name}*")],
                end: cursor,
                terminator: VerbatimTerminator::AnyPosition,
            })
        }
        // codehigh.sty creates each starred or unstarred name separately.
        "NewCodeHighEnv" => {
            let name = literal_name(argument(source, &mut cursor)?)?;
            argument(source, &mut cursor)?;
            Some(VerbatimDeclaration {
                names: vec![name.into()],
                end: cursor,
                terminator: VerbatimTerminator::AnyPosition,
            })
        }
        // The console family adds a `sole` suffix to names such as `pycon`.
        "makepythontexfamily" => {
            let (family, console) = pythontex_family(source, &mut cursor)?;
            let names = [
                "code",
                "verbatim",
                "sub",
                if console { "sole" } else { "block" },
            ]
            .into_iter()
            .map(|suffix| format!("{family}{suffix}"))
            .collect();
            Some(VerbatimDeclaration {
                names,
                end: cursor,
                terminator: VerbatimTerminator::IndentedLine,
            })
        }
        _ => None,
    }
}
