//! Delimited package arguments that must stay opaque to outer row/column scans.

use crate::syntax;
use crate::trivia::skip as trivia;

/// Citation arguments and bibliography customization commands from biblatex.
/// Explicit user declarations replace these defaults through the normal table.
pub(crate) fn default_biblatex_commands()
-> std::collections::BTreeMap<String, crate::config::CommandRule> {
    use crate::config::CommandRule;
    let mut commands = std::collections::BTreeMap::new();
    for name in [
        "cite",
        "Cite",
        "parencite",
        "Parencite",
        "footcite",
        "footcitetext",
        "textcite",
        "Textcite",
        "smartcite",
        "Smartcite",
        "autocite",
        "Autocite",
        "supercite",
        "fullcite",
        "footfullcite",
        "citeauthor",
        "Citeauthor",
        "citetitle",
        "citeyear",
        "citeyearpar",
        "citedate",
        "citeurl",
        "volcite",
        "pvolcite",
        "fvolcite",
        "ftvolcite",
        "svolcite",
        "tvolcite",
        "avolcite",
    ] {
        commands.insert(
            name.into(),
            CommandRule {
                arguments: Some(if name.contains("volcite") { 2 } else { 1 }),
                optional_arguments: true,
                ..Default::default()
            },
        );
    }
    for name in [
        "cites",
        "Cites",
        "parencites",
        "Parencites",
        "footcites",
        "footcitetexts",
        "textcites",
        "Textcites",
        "smartcites",
        "Smartcites",
        "autocites",
        "Autocites",
        "supercites",
    ] {
        commands.insert(
            name.into(),
            CommandRule {
                arguments: None,
                optional_arguments: true,
                ..Default::default()
            },
        );
    }
    for (name, count, optional) in [
        ("addbibresource", 1, true),
        ("addglobalbib", 1, true),
        ("addsectionbib", 1, true),
        ("nocite", 1, false),
        ("printbibliography", 0, true),
        ("printbibheading", 0, true),
        ("printbiblist", 1, true),
        ("ExecuteBibliographyOptions", 1, true),
        ("DeclareBibliographyDriver", 2, false),
        ("DeclareFieldFormat", 2, true),
        ("DeclareNameFormat", 2, true),
        ("DeclareListFormat", 2, true),
        ("DeclareIndexFieldFormat", 2, true),
        ("DeclareIndexNameFormat", 2, true),
        ("DeclareIndexListFormat", 2, true),
        ("DeclareBibliographyAlias", 2, false),
        ("DeclareFieldAlias", 2, true),
        ("DeclareNameAlias", 2, true),
        ("DeclareListAlias", 2, true),
        ("defbibheading", 2, true),
        ("defbibenvironment", 4, false),
        ("defbibfilter", 2, false),
        ("defbibcheck", 2, false),
        ("defbibnote", 2, false),
        ("DeclareSourcemap", 1, false),
        ("DeclareStyleSourcemap", 1, false),
        ("DeclareSortingTemplate", 2, false),
        ("DeclareRefcontext", 2, false),
        // biblatex.sty:10464--10553 accepts [keys] and an optional braced name.
        // This table limits adjacent braced arguments without requiring them.
        ("newrefcontext", 1, true),
        ("localrefcontext", 1, true),
        ("endrefcontext", 0, false),
        ("assignrefcontextcats", 1, true),
        ("assignrefcontextentries", 1, true),
        ("assignrefcontextkeyws", 1, true),
        // Annotation predicates consume their true/false branches; accessors
        // only take bracketed field/annotation-name options.
        ("iffieldannotation", 3, true),
        ("hasfieldannotation", 2, true),
        ("getfieldannotation", 0, true),
        ("AtEveryBibitem", 1, false),
        ("AtEveryCitekey", 1, false),
        ("AtBeginBibliography", 1, false),
    ] {
        commands.insert(
            name.into(),
            CommandRule {
                arguments: Some(count),
                optional_arguments: optional,
                ..Default::default()
            },
        );
    }
    commands
}

fn default_package_commands() -> std::collections::BTreeMap<String, crate::config::CommandRule> {
    use crate::config::CommandRule;
    let mut commands = default_biblatex_commands();
    super::add_extra_commands(&mut commands);
    for (name, shape) in [
        // natbib citations read two optional notes before the citation keys.
        ("citet", "oom"),
        ("citep", "oom"),
        ("citeauthor", "oom"),
        ("citeyear", "oom"),
        // etoolbox targets are single TeX tokens or braced control sequences.
        ("patchcmd", "ommmmm"),
        ("apptocmd", "ommmm"),
        ("pretocmd", "ommmm"),
        ("ifpatchable", "mmmm"),
        ("appto", "mm"),
        ("preto", "mm"),
    ] {
        commands.insert(
            name.into(),
            CommandRule {
                arguments: Some(shape.bytes().filter(|&slot| slot == b'm').count()),
                optional_arguments: shape.contains('o'),
                ..Default::default()
            },
        );
    }
    for (name, count, optional) in [
        // adjustbox commands collect their final content argument.
        ("adjustbox", 2, false),
        ("adjustboxset", 1, false),
        ("adjustimage", 2, false),
        ("adjincludegraphics", 1, true),
        ("minsizebox", 3, false),
        ("maxsizebox", 3, false),
        // mdframed and changepage configuration.
        ("mdfsetup", 1, false),
        ("mdfdefinestyle", 2, false),
        ("changepage", 9, false),
        ("changetext", 5, false),
        ("checkoddpage", 0, false),
        // multicol's explicit break supports an optional break penalty.
        ("columnbreak", 0, true),
        // tasks configuration and an explicit new-row command.
        ("settasks", 1, false),
        ("startnewitemline", 0, false),
        // csquotes plain, text and integrated citation variants.
        ("enquote", 1, false),
        ("foreignquote", 2, false),
        ("hyphenquote", 2, false),
        ("textquote", 1, true),
        ("blockquote", 1, true),
        ("foreigntextquote", 2, true),
        ("hyphentextquote", 2, true),
        ("foreignblockquote", 2, true),
        ("hyphenblockquote", 2, true),
        ("textcquote", 2, true),
        ("blockcquote", 2, true),
        ("foreigntextcquote", 3, true),
        ("hyphentextcquote", 3, true),
        ("foreignblockcquote", 3, true),
        ("hyphenblockcquote", 3, true),
        // pdfpages keeps page/merge options attached to the filename argument.
        ("includepdf", 1, true),
        ("includepdfmerge", 1, true),
        // soul's text decorators consume one text argument. Following braces
        // belong to the surrounding paragraph, rather than another argument.
        ("ul", 1, false),
        ("st", 1, false),
        ("hl", 1, false),
        ("so", 1, false),
        ("caps", 1, false),
        ("setul", 2, false),
        ("setulcolor", 1, false),
        ("setstcolor", 1, false),
        ("sethlcolor", 1, false),
    ] {
        commands.insert(
            name.into(),
            CommandRule {
                arguments: Some(count),
                optional_arguments: optional,
                ..Default::default()
            },
        );
    }
    commands
}

pub(crate) fn builtin_command_rule(name: &str) -> Option<&'static crate::config::CommandRule> {
    static COMMANDS: std::sync::OnceLock<
        std::collections::BTreeMap<String, crate::config::CommandRule>,
    > = std::sync::OnceLock::new();
    COMMANDS
        .get_or_init(default_package_commands)
        .get(name.trim_end_matches('*'))
        .or_else(|| crate::document_environments::command_rule(name))
}

fn separated_star(source: &str, name: &str, end: usize) -> bool {
    !name.ends_with('*') && source.as_bytes().get(trivia(source, end)) == Some(&b'*')
}

/// Ordered citation notes and delayed macro-patching arguments.
pub(crate) fn command_signature(source: &str, name: &str, end: usize) -> Option<&'static str> {
    if let Some(shape) = stored_kernel_signature(name) {
        return Some(shape);
    }
    let separated = separated_star(source, name, end);
    Some(match name.trim_end_matches('*') {
        "citet" | "citep" | "citeauthor" | "citeyear" => {
            if separated {
                "moom"
            } else {
                "oom"
            }
        }
        "patchcmd" => "ommmmm",
        "apptocmd" | "pretocmd" => "ommmm",
        "appto" | "preto" => "mm",
        "ifpatchable" if separated => "mmmm",
        "ifpatchable" if name.ends_with('*') => "mmm",
        "ifpatchable" => "mmmm",
        // etoolbox.sty:130ff: one tested token/name, then two branch arguments.
        "ifundef" | "ifcsundef" | "ifdef" | "ifcsdef" | "ifdefvoid" | "ifcsvoid" | "ifdefempty"
        | "ifcsempty" | "ifblank" | "ifstrempty" | "iftoggle" | "ifbool" => "mmm",
        "ifstrequal" | "ifdefequal" | "ifcsequal" | "ifdefstring" | "ifcsstring" => "mmmm",
        "ifnumcomp" | "ifdimcomp" => "mmmmm",
        // KOMA-Script TOC hooks; titling.sty title/author/date hooks.
        "BeforeStartingTOC" | "AfterStartingTOC" => "om",
        "pretitle" | "posttitle" | "preauthor" | "postauthor" | "predate" | "postdate" => "m",
        // doc.sty:380--388 reads a comma list of control-sequence names.
        "regexp" | "dirname" | "DoNotIndex" => "m",
        "lily" | "lilypond" => "om",
        // lyluatex.sty:254--280 stores the display wrappers as source data;
        // lyenv only changes optional-argument dispatch for a literal name.
        "lysetverbenv" => "mm",
        "lyenv" => "m",
        _ => return subcaption_command_signature(source, name, end),
    })
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    stored_kernel_signature(name).is_some()
        || matches!(
            name.trim_end_matches('*'),
            "patchcmd"
                | "apptocmd"
                | "pretocmd"
                | "appto"
                | "preto"
                | "ifpatchable"
                | "BeforeStartingTOC"
                | "AfterStartingTOC"
                | "pretitle"
                | "posttitle"
                | "preauthor"
                | "postauthor"
                | "predate"
                | "postdate"
                | "regexp"
                | "dirname"
                | "DoNotIndex"
                | "lily"
                | "lilypond"
                | "lysetverbenv"
                | "lyenv"
        )
}

/// Recording sinks consume source data without executing a supplied TeX body.
/// Deferred hooks and patches have a different effect even though their stored
/// arguments are opaque while the registration command itself is parsed.
pub(crate) fn is_recording_command(name: &str) -> bool {
    matches!(name, "lily" | "lilypond")
}

/// Search/replacement tokens and appended code retain their source bytes.
/// Immediate success/failure branches remain available for nested formatting.
pub(crate) fn command_header_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if let Some(shape) = stored_kernel_signature(name) {
        return crate::syntax::arguments::signature_end(source, end, shape);
    }
    let shape = match name.trim_end_matches('*') {
        "patchcmd" => "ommm",
        "apptocmd" | "pretocmd" => "omm",
        "appto" | "preto" => "mm",
        "ifpatchable" if separated_star(source, name, end) => "mm",
        "ifpatchable" if name.ends_with('*') => "m",
        "ifpatchable" => "mm",
        "BeforeStartingTOC" | "AfterStartingTOC" => "om",
        "pretitle" | "posttitle" | "preauthor" | "postauthor" | "predate" | "postdate"
        | "regexp" | "dirname" | "DoNotIndex" => "m",
        "lily" | "lilypond" => "om",
        "lysetverbenv" => "mm",
        "lyenv" => "m",
        _ => return subcaption_command_header_end(source, name, end),
    };
    // Incomplete calls do not establish a metadata boundary.
    crate::syntax::arguments::signature_end(source, end, command_signature(source, name, end)?)?;
    crate::syntax::arguments::signature_end(source, end, shape).filter(|&finish| finish > end)
}

fn stored_kernel_signature(name: &str) -> Option<&'static str> {
    match name {
        // The kernel stores this control-sequence name in its preamble list.
        "@onlypreamble" => Some("m"),
        _ => None,
    }
}

fn mandatory_argument_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => syntax::balanced_end(source, start, b'{', b'}'),
        b'\\' => syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' => None,
        _ => source[start..].chars().next().map(|c| start + c.len_utf8()),
    }
}

/// Caption metadata stays opaque while the final content argument remains
/// available for ordinary group and nested-environment formatting.
pub(crate) fn subcaption_command_signature(
    source: &str,
    name: &str,
    end: usize,
) -> Option<&'static str> {
    Some(match name {
        "subfigure" | "subtable" | "subfloat" => "oom",
        "subcaptionbox*" => "moom",
        "subcaptionbox" if source.as_bytes().get(trivia(source, end)) == Some(&b'*') => "mmoom",
        "subcaptionbox" => "omoom",
        _ => return None,
    })
}

/// The protected prefix stops before the final printable content argument.
pub(crate) fn subcaption_command_header_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let mut cursor = end;
    match name {
        // subfigure.sty:210--236 and subfig.sty:319--354 take at most two
        // bracketed list-entry/caption arguments before the content token.
        "subfigure" | "subtable" | "subfloat" => {
            for _ in 0..2 {
                let start = trivia(source, cursor);
                if source.as_bytes().get(start) != Some(&b'[') {
                    break;
                }
                cursor = syntax::balanced_end(source, start, b'[', b']')?;
            }
            if cursor == end {
                return None;
            }
        }
        // subcaption.dtx:237--238; caption.sty:454--470 reads
        // s o m o o m. The starred variant omits the list entry.
        "subcaptionbox" | "subcaptionbox*" => {
            let mut starred = name.ends_with('*');
            let start = trivia(source, cursor);
            if !starred && source.as_bytes().get(start) == Some(&b'*') {
                starred = true;
                cursor = start + 1;
            }
            let start = trivia(source, cursor);
            if !starred && source.as_bytes().get(start) == Some(&b'[') {
                cursor = syntax::balanced_end(source, start, b'[', b']')?;
            }
            cursor = mandatory_argument_end(source, trivia(source, cursor))?;
            for _ in 0..2 {
                let start = trivia(source, cursor);
                if source.as_bytes().get(start) != Some(&b'[') {
                    break;
                }
                cursor = syntax::balanced_end(source, start, b'[', b']')?;
            }
        }
        _ => return None,
    }
    mandatory_argument_end(source, trivia(source, cursor))?;
    Some(cursor)
}

fn delimited_end(source: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&open) {
        return None;
    }
    let mut cursor = start + 1;
    let mut depth = 1;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'\\' => cursor = syntax::command_at(source, cursor)?.1,
            b'%' => {
                cursor = source[cursor..]
                    .find('\n')
                    .map_or(source.len(), |i| cursor + i + 1)
            }
            b'{' => cursor = syntax::balanced_end(source, cursor, b'{', b'}')?,
            byte if byte == close => {
                depth -= 1;
                cursor += 1;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            byte if byte == open => {
                depth += 1;
                cursor += 1;
            }
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    None
}

/// physics.sty:75,109 reads `s g o d() d||`. Internal alignment tokens
/// belong to those arguments even when they have no surrounding brace group.
pub(crate) fn physics_matrix_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if !matches!(
        name.trim_end_matches('*'),
        "mqty" | "matrixquantity" | "smqty" | "smallmatrixquantity"
    ) {
        return None;
    }
    let mut cursor = end;
    let star = trivia(source, cursor);
    if !name.ends_with('*') && source.as_bytes().get(star) == Some(&b'*') {
        cursor = star + 1;
    }
    let mut found = false;
    for (open, close) in [(b'{', b'}'), (b'[', b']'), (b'(', b')'), (b'|', b'|')] {
        let start = trivia(source, cursor);
        if source.as_bytes().get(start) != Some(&open) {
            continue;
        }
        cursor = if open == b'{' || open == b'[' {
            syntax::balanced_end(source, start, open, close)?
        } else {
            delimited_end(source, start, open, close)?
        };
        found = true;
    }
    found.then_some(cursor)
}
