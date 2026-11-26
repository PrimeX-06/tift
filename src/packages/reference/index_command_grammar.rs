//! Index entry data and public interfaces shared by the usual index packages.
//!
//! makeidx, imakeidx, indextools, splitidx and index use overlapping names.
//! Their common optional forms keep following document groups outside commands.
//! Longer mandatory variants remain available through source/user declarations.

use crate::config::CommandRule;
use std::collections::BTreeMap;

macro_rules! command_signatures {
    ($( $($name:literal)|+ => $signature:literal ),+ $(,)?) => {
        fn fixed_signature(name: &str) -> Option<&'static str> {
            Some(match name.trim_end_matches('*') {
                $( $($name)|+ => $signature, )+
                _ => return None,
            })
        }

        pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
            $(
                let arguments = $signature.bytes().filter(|&slot| slot == b'm').count();
                let optional_arguments = $signature.contains('o');
                for name in [$($name),+] {
                    commands.insert(name.into(), CommandRule {
                        arguments: Some(arguments),
                        optional_arguments,
                        ..Default::default()
                    });
                }
            )+
        }
    };
}

command_signatures! {
    // makeidx:48; imakeidx:72,264; indextools:91,295; index:253--261;
    // splitidx:204--246. Plain makeidx printing consumes no brace argument.
    "makeindex" => "o",
    "printindex" | "printsubindex" => "oo",
    // imakeidx:140--170; indextools:163--193; index:122--162;
    // splitidx:98--129. The entry is read after changing special catcodes.
    "index" | "sindex" => "om",
    "indexentry" => "omm",
    // forest-index.sty registers the supplied control sequence for memoization.
    "forest@memoize" => "m",
    // splitidx:259,308; index:46--111. The shared minimum is one argument;
    // index's four-argument newindex form must not absorb arbitrary body groups.
    "newindex" | "newprotectedindex" => "om",
    "renewindex" => "ommmm",
    "disableindex" => "m",
    // imakeidx:59,258,261; indextools:72,289,292.
    "splitindexoptions" | "indexsetup" => "m",
    "indexprologue" => "om",
    // splitidx:189--202: g@addto@macro reads the second, delayed code argument.
    "AtWriteToIndex" | "AtNextWriteToIndex" => "mm",
    "extendtheindex" => "mmmm",
    "setindexpreamble" => "om",
    "useindexpreamble" => "o",
    "printindices" => "",
    // idxlayout:81,94,96,103,130--138,220. showidx keeps the kernel index form.
    "idxlayout" | "setindexprenote" | "indexstheadcase" => "m",
    "noindexprenote" | "indexfont" | "indexjustific" | "indexsubsdelim"
        | "indexspace" | "seename" | "alsoname" | "indexname" => "",
    // makeidx:46--47; imakeidx:67--68; splitidx:94--95; index:188.
    "see" | "seealso" => "mm",
}

fn next_token(source: &str, cursor: usize) -> Option<u8> {
    source
        .as_bytes()
        .get(crate::trivia::skip(source, cursor))
        .copied()
}

fn after_optional(source: &str, cursor: usize) -> Option<usize> {
    let start = crate::trivia::skip(source, cursor);
    if source.as_bytes().get(start) == Some(&b'[') {
        crate::syntax::balanced_end(source, start, b'[', b']')
    } else {
        Some(cursor)
    }
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    let fixed = fixed_signature(name)?;
    let base = name.trim_end_matches('*');
    match base {
        "printindex" | "printsubindex" if name.ends_with('*') => Some(""),
        "printindex" | "printsubindex" if next_token(source, cursor) == Some(b'*') => Some("m"),
        "index" if !name.ends_with('*') && next_token(source, cursor) == Some(b'*') => Some("mom"),
        "newindex" | "renewindex"
            if !name.ends_with('*')
                && next_token(source, after_optional(source, cursor)?) == Some(b'*') =>
        {
            Some(if base == "newindex" { "omm" } else { "ommmmm" })
        }
        _ => Some(fixed),
    }
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    fixed_signature(name).is_some()
}

fn mandatory_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => crate::syntax::balanced_end(source, start, b'{', b'}'),
        b'\\' => crate::syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' | b']' => None,
        _ => source[start..].chars().next().map(|c| start + c.len_utf8()),
    }
}

/// @sanitize changes backslash and percent to ordinary characters while braces
/// retain their group catcodes. Escaped braces and percent comments consequently
/// follow the raw byte grammar of the index entry, not the document grammar.
fn sanitized_brace_end(source: &str, start: usize) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&b'{') {
        return None;
    }
    let mut depth = 1usize;
    let mut cursor = start + 1;
    while cursor < source.len() {
        cursor += memchr::memchr2(b'{', b'}', &source.as_bytes()[cursor..])?;
        if source.as_bytes()[cursor] == b'{' {
            depth += 1;
        } else {
            depth -= 1;
            if depth == 0 {
                return Some(cursor + 1);
            }
        }
        cursor += 1;
    }
    None
}

fn sanitized_optional_end(source: &str, start: usize) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[start + 1..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.checked_sub(1)?,
            b']' if depth == 0 => return Some(start + offset + 2),
            _ => (),
        }
    }
    None
}

fn entry_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let mut end = cursor;
    if name.trim_end_matches('*') == "index"
        && !name.ends_with('*')
        && next_token(source, end) == Some(b'*')
    {
        end = crate::trivia::skip(source, end) + 1;
    }
    let start = crate::trivia::skip(source, end);
    if source.as_bytes().get(start) == Some(&b'[') {
        let raw = sanitized_optional_end(source, start)?;
        // index.sty reads the name before @sanitize; the other providers read
        // it afterwards. Prefer a braced raw entry, otherwise retain ordinary
        // optional comments for the unsanitized name form.
        end = if source.as_bytes().get(crate::trivia::skip(source, raw)) == Some(&b'{') {
            raw
        } else {
            crate::syntax::balanced_end(source, start, b'[', b']')?
        };
    }
    let start = crate::trivia::skip(source, end);
    let finish = if source.as_bytes().get(start) == Some(&b'{') {
        sanitized_brace_end(source, start)?
    } else {
        // A sanitized backslash is a single character, not a control word.
        match source.as_bytes().get(start)? {
            b'}' | b']' => return None,
            _ => start + source[start..].chars().next()?.len_utf8(),
        }
    };
    Some(finish)
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    if matches!(name.trim_end_matches('*'), "index" | "sindex") {
        return entry_header_end(source, name, cursor);
    }
    if name.trim_end_matches('*') == "indexentry" {
        let entry = entry_header_end(source, name, cursor)?;
        return mandatory_end(source, crate::trivia::skip(source, entry));
    }
    let signature = command_signature(source, name, cursor)?;
    let mut end = cursor;
    for slot in signature.bytes() {
        let start = crate::trivia::skip(source, end);
        match slot {
            b'o' if source.as_bytes().get(start) == Some(&b'[') => {
                end = crate::syntax::balanced_end(source, start, b'[', b']')?;
            }
            b'o' => (),
            b'm' => end = mandatory_end(source, start)?,
            _ => return None,
        }
    }
    (end > cursor).then_some(end)
}

/// A configuration header uses ordinary TeX whitespace and grouping. Parse a
/// bounded view for indentation while its original range stays opaque to
/// declaration inference, wrapping and alignment. Raw readers and dynamic
/// tokenization keep the complete header unchanged.
pub(crate) fn configuration_layout(
    source: &str,
    name: &str,
    config: &crate::CompiledConfig,
) -> Option<Vec<crate::syntax::SharedBlock>> {
    if !matches!(
        name,
        "makeindex" | "indexsetup" | "splitindexoptions" | "idxlayout"
    ) || !source.contains('\n')
    {
        return None;
    }
    for (start, _) in source.match_indices('\\') {
        if crate::syntax::is_escaped(source, start) {
            continue;
        }
        if let Some((command, _)) = crate::syntax::command_at(source, start)
            && (config
                .config
                .protect
                .commands
                .iter()
                .any(|name| name == command)
                || config
                    .config
                    .syntax
                    .commands
                    .get(command)
                    .is_some_and(|rule| rule.verbatim || rule.preserve_body)
                || matches!(
                    command.trim_end_matches('*'),
                    "verb"
                        | "Verb"
                        | "lstinline"
                        | "mintinline"
                        | "detokenize"
                        | "unexpanded"
                        | "catcode"
                        | "scantokens"
                        | "obeyspaces"
                        | "obeylines"
                        | "ExplSyntaxOn"
                        | "begin"
                        | "end"
                        | "csname"
                        | "expandafter"
                        | "bgroup"
                        | "egroup"
                        | "begingroup"
                        | "endgroup"
                        | "index"
                        | "sindex"
                        | "indexentry"
                        | "url"
                        | "nolinkurl"
                        | "path"
                        | "href"
                        | "hyperimage"
                        | "hyperbaseurl"
                        | "input"
                        | "include"
                        | "includegraphics"
                ))
        {
            return None;
        }
    }
    let (_, blocks) = crate::syntax::shared_blocks_with_ranges(source, config, &[]).ok()?;
    blocks
        .first()
        .filter(|block| block.kind == crate::syntax::BlockKind::Command && block.name == name)?;
    Some(blocks)
}
