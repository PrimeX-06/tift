//! Packages that collect their source through tcolorbox or listings.

use super::InvocationContext;

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[
    "codeview",
    "codeview*",
    "termcode",
    "highlightblock",
    "hllblock",
];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    match name {
        // codebox.sty:346,376 and terminalcode.sty:222.
        "codeview" | "codeview*" | "termcode" => Some("om"),
        // highlightlatex.sty:188,199.
        "highlightblock" | "hllblock" => Some("o"),
        _ => None,
    }
}

pub(crate) fn is_terminal_listing(name: &str) -> bool {
    matches!(
        name,
        "TermUnix" | "TermWin" | "TermMac" | "TerminalUnix" | "TerminalWin" | "TerminalMac"
    )
}

pub(crate) fn context_header_signature(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    match name {
        // forest-doc.sty displays these lines through listings and saves a copy.
        "forestexample" if context.has_package("forest-doc") => Some("o"),
        "numVblock" if context.has_package("numberedblock") => Some("o"),
        "codeexample" if context.has_package("pgfmanual-en-macros") => Some("o"),
        _ if is_terminal_listing(name) && context.has_package("sim-os-menus") => Some("om"),
        // naive-ebnf tokenizes punctuation in the collected grammar body.
        "ebnf" if context.has_package("naive-ebnf") => Some("o"),
        "example" | "sidebyside" | "sourcecode" | "bash"
            if context.has_package("cnltx-example") =>
        {
            Some("o")
        }
        "externalDocument" if context.has_package("hvextern") => Some("om"),
        "exdispExample" if context.has_package("tcolorbox.doc.s_main") => Some("om"),
        "exdispExample*" if context.has_package("tcolorbox.doc.s_main") => Some("mm"),
        "ltxexample" | "ltxcode" if context.has_package("ltxdockit") => Some("o"),
        // clrscode3e.sty:223 also defines codebox, as ordinary TeX.
        "codebox" | "codebox*" if context.has_package("codebox") => Some("om"),
        // The second listings parameter consumes a line-end token in the
        // normal block form, so it is not a source header argument.
        "python" if context.has_package("pythonhighlight") => Some("o"),
        "python" if context.has_package("python") => Some("o"),
        // minted-code.sty:38--98 supplies these generic names via newtcblisting.
        "codeblock" | "codeblockA" if context.has_package("minted-code") => Some("o"),
        "exampleA" | "exampleB" | "exampleC" | "exampleM" if context.has_package("minted-code") => {
            Some("om")
        }
        _ => None,
    }
}

pub(crate) fn inline_end(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<usize> {
    let mut cursor = crate::trivia::skip(source, cursor);
    if name == "inlinecode" && context.has_package("pdfToolbox") {
        let delimiter = source.get(cursor..)?.chars().next()?;
        cursor += delimiter.len_utf8();
        return source[cursor..]
            .find(delimiter)
            .map(|offset| cursor + offset + delimiter.len_utf8());
    }
    if name == "KKverb" && (context.has_package("kkluaverb") || context.has_package("KKluaverb")) {
        if source.as_bytes().get(cursor) != Some(&b'|') {
            return None;
        }
        cursor += 1;
        return source[cursor..].find('|').map(|offset| cursor + offset + 1);
    }
    match name {
        // highlightlatex.sty:209 forwards options; pythonhighlight.sty:158
        // already supplies a fixed style option to listings.
        "hll" if context.has_package("highlightlatex") => {
            if source.as_bytes().get(cursor) == Some(&b'[') {
                cursor = crate::trivia::skip(
                    source,
                    crate::syntax::balanced_end(source, cursor, b'[', b']')?,
                );
            }
        }
        "pyth" if context.has_package("pythonhighlight") => {}
        _ => return None,
    }
    let delimiter = source.get(cursor..)?.chars().next()?;
    let close = if delimiter == '{' { '}' } else { delimiter };
    let body = cursor + delimiter.len_utf8();
    let line = source[body..].split(['\r', '\n']).next()?;
    line.find(close)
        .map(|offset| body + offset + close.len_utf8())
}
