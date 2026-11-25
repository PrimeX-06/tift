//! Layout of classic BibTeX's postfix style language.

use std::ops::Range;

use crate::syntax::FormatDirectiveState;
use crate::{CompiledConfig, Diagnostic, FormatError, FormatOutput, Severity};

fn split_line(line: &str) -> (&str, &str) {
    let body = line
        .strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(line);
    (body, &line[body.len()..])
}

/// Both quotes must occur on the same physical line. A backslash is ordinary
/// string data, even immediately before the closing quote.
fn lexemes(line: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rest = line;
    std::iter::from_fn(move || {
        rest = rest.trim_start_matches([' ', '\t']);
        let offset = line.len() - rest.len();
        let end = match rest.as_bytes().first()? {
            b'%' => rest.len(),
            b'{' | b'}' => 1,
            b'"' => rest[1..].find('"').map_or(rest.len(), |at| at + 2),
            _ => rest
                .find([' ', '\t', '{', '}', '"', '%'])
                .unwrap_or(rest.len()),
        };
        let token;
        (token, rest) = rest.split_at(end);
        Some((offset, token))
    })
}

pub(crate) fn protected_ranges(source: &str, include_literals: bool) -> Vec<Range<usize>> {
    if !include_literals && !(source.contains("tift:") || source.contains("tex-fmt:")) {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut directives = FormatDirectiveState::default();
    let mut offset = 0;
    let mut skip_until = 0;
    for line in source.split_inclusive('\n') {
        let end = offset + line.len();
        if offset >= skip_until {
            for (at, token) in lexemes(split_line(line).0) {
                let start = offset + at;
                if token.starts_with('%') {
                    skip_until = directives.observe(source, start, end, &mut ranges);
                }
                if include_literals && token.starts_with(['%', '"']) {
                    ranges.push(start..start + token.len());
                }
            }
        }
        offset = end;
    }
    directives.finish(source.len(), &mut ranges);
    ranges.sort_unstable_by_key(|range| range.start);
    ranges.dedup_by(|later, earlier| {
        if later.start <= earlier.end {
            earlier.end = earlier.end.max(later.end);
            true
        } else {
            false
        }
    });
    ranges
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Argument {
    Names,
    Name,
    String,
    Body,
    Anonymous,
}

fn arguments(command: &str) -> Option<&'static [Argument]> {
    use Argument::*;
    Some(match command.to_ascii_uppercase().as_str() {
        "ENTRY" => &[Names, Names, Names],
        "INTEGERS" | "STRINGS" => &[Names],
        "FUNCTION" => &[Name, Body],
        "MACRO" => &[Name, String],
        "EXECUTE" | "ITERATE" | "REVERSE" => &[Name],
        "READ" | "SORT" => &[],
        _ => return None,
    })
}

fn identifier(word: &str) -> bool {
    !word.is_empty()
        && !word.as_bytes()[0].is_ascii_digit()
        && !word.bytes().any(|b| {
            b.is_ascii_control()
                || matches!(
                    b,
                    b'"' | b'#' | b'%' | b'\'' | b'(' | b')' | b',' | b'=' | b'{' | b'}'
                )
        })
}

fn instruction(word: &str) -> bool {
    if let Some(integer) = word.strip_prefix('#') {
        let digits = integer.strip_prefix('-').unwrap_or(integer);
        return !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    }
    let name = word.strip_prefix('\'').unwrap_or(word);
    identifier(name) || matches!(name, ":=" | "=")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gap {
    Tight,
    Space,
    Line,
}

struct Token<'a> {
    text: &'a str,
    offset: usize,
    line: usize,
    depth: usize,
    before: Gap,
    after: Gap,
    // On a closing brace, the start of the anonymous function pushed onto the stack.
    anonymous_open: Option<usize>,
}

struct Group {
    open: usize,
    argument: Argument,
    has_value: bool,
}

fn error(span: Range<usize>, message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        span,
        message: format!("BibTeX style: {message}"),
    }
}

fn parse<'a>(source: &'a str, opaque: &[Range<usize>]) -> Result<Vec<Token<'a>>, Diagnostic> {
    let mut tokens: Vec<Token<'a>> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut pending: &[Argument] = &[];
    let mut offset = 0;
    for (line_number, line) in source.split_inclusive('\n').enumerate() {
        let start = offset;
        offset += line.len();
        if crate::syntax::is_protected(start, opaque) {
            continue;
        }
        for (column, word) in lexemes(split_line(line).0) {
            let at = start + column;
            let invalid = |message: &str| error(at..at + word.len(), message);
            let mut token = Token {
                text: word,
                offset: at,
                line: line_number,
                depth: groups.len(),
                before: Gap::Space,
                after: Gap::Space,
                anonymous_open: None,
            };
            if word.starts_with('"') && (word.len() < 2 || !word.ends_with('"')) {
                return Err(invalid("unterminated string on this line"));
            }
            // BibTeX accepts a right brace or comment immediately after a
            // stack instruction, but a following literal or left brace needs whitespace.
            if !groups.is_empty()
                && !word.starts_with(['}', '%'])
                && tokens.last().is_some_and(|prev| {
                    prev.offset + prev.text.len() == at && !matches!(prev.text, "{" | "}")
                })
            {
                return Err(invalid("missing separator between tokens"));
            }
            if word == "{" {
                let argument = if let Some(group) = groups.last() {
                    if !matches!(group.argument, Argument::Body | Argument::Anonymous) {
                        return Err(invalid("unexpected nested group in command argument"));
                    }
                    Argument::Anonymous
                } else {
                    let Some((&argument, rest)) = pending.split_first() else {
                        return Err(invalid("unexpected opening brace"));
                    };
                    pending = rest;
                    argument
                };
                groups.push(Group {
                    open: tokens.len(),
                    argument,
                    has_value: false,
                });
            } else if word == "}" {
                let Some(group) = groups.pop() else {
                    return Err(invalid("unexpected closing brace"));
                };
                if matches!(group.argument, Argument::Name | Argument::String) && !group.has_value {
                    return Err(invalid("missing name or string in command argument"));
                }
                let empty = group.open + 1 == tokens.len();
                let open = &mut tokens[group.open];
                let gap = if open.line != line_number || group.argument == Argument::Body && !empty
                {
                    Gap::Line
                } else if group.argument == Argument::Anonymous && !empty {
                    Gap::Space
                } else {
                    Gap::Tight
                };
                open.after = gap;
                token.before = gap;
                token.depth = groups.len();
                if group.argument == Argument::Anonymous {
                    token.anonymous_open = Some(group.open);
                }
            } else if !word.starts_with('%') {
                if let Some(group) = groups.last_mut() {
                    let valid = match group.argument {
                        Argument::Name => !group.has_value && identifier(word),
                        Argument::Names => identifier(word),
                        Argument::String => !group.has_value && word.starts_with('"'),
                        Argument::Body | Argument::Anonymous => {
                            word.starts_with('"') || instruction(word)
                        }
                    };
                    if !valid {
                        return Err(invalid(
                            "invalid name, literal, or operand for this argument",
                        ));
                    }
                    group.has_value = true;
                } else {
                    if !pending.is_empty() {
                        return Err(invalid("expected a braced command argument"));
                    }
                    pending = arguments(word).ok_or_else(|| invalid("unknown style command"))?;
                    token.before = Gap::Line;
                }
            }
            tokens.push(token);
        }
    }
    if let Some(group) = groups.last() {
        let at = tokens[group.open].offset;
        return Err(error(at..at + 1, "unclosed group"));
    }
    if !pending.is_empty() {
        return Err(error(
            source.len()..source.len(),
            "missing braced argument at end of input",
        ));
    }
    Ok(tokens)
}

/// The operand immediately preceding an instruction must be a quoted name or
/// a complete anonymous function. Ordinary instructions may compute arbitrary
/// stack values, so branch indentation stops at such an instruction.
fn preceding_branch(tokens: &[Token<'_>], end: usize) -> Option<usize> {
    let index = tokens[..end]
        .iter()
        .rposition(|token| !token.text.starts_with('%'))?;
    let token = &tokens[index];
    token
        .anonymous_open
        .or_else(|| token.text.starts_with('\'').then_some(index))
}

fn layout_functions(tokens: &mut [Token<'_>], config: &CompiledConfig) {
    // Nested branch ranges overlap. Accumulate their depth changes in one pass.
    let mut branch_depth_changes = vec![0isize; tokens.len()];
    for index in 0..tokens.len() {
        if tokens[index].depth > 0
            && tokens[index].text.eq_ignore_ascii_case("if$")
            && let Some(second) = preceding_branch(tokens, index)
            && let Some(first) = preceding_branch(tokens, second)
            && (tokens[first].line != tokens[index].line
                || first.checked_sub(1).is_some_and(|previous| {
                    tokens[previous].line < tokens[first].line && tokens[previous].text != "{"
                }))
        {
            tokens[first].before = Gap::Line;
            tokens[second].before = Gap::Line;
            tokens[index].before = Gap::Line;
            branch_depth_changes[first] += 1;
            branch_depth_changes[index] -= 1;
        }
        if tokens[index].text.eq_ignore_ascii_case("FUNCTION") && tokens[index].depth == 0 {
            compact_string_function(&mut tokens[index..], config);
        }
    }
    let mut extra_depth = 0;
    for (token, change) in tokens.iter_mut().zip(branch_depth_changes) {
        extra_depth += change;
        token.depth += extra_depth as usize;
    }
}

fn compact_string_function(tokens: &mut [Token<'_>], config: &CompiledConfig) {
    let [
        command,
        name_open,
        name,
        name_close,
        body_open,
        literal,
        body_close,
        rest @ ..,
    ] = tokens
    else {
        return;
    };
    if command.line != body_close.line
        || [
            name_open.text,
            name_close.text,
            body_open.text,
            body_close.text,
        ] != ["{", "}", "{", "}"]
        || !literal.text.starts_with('"')
        || rest
            .first()
            .is_some_and(|next| next.line == command.line && next.text.starts_with('%'))
    {
        return;
    }
    let line = format!("{} {{{}}} {{ {} }}", command.text, name.text, literal.text);
    if crate::wrap::display_width(&line, 0, config.config.indent.width)
        <= config.config.wrap.columns
    {
        body_open.after = Gap::Space;
        body_close.before = Gap::Space;
    }
}

fn write_indent(output: &mut String, unit: &str, depth: usize, config: &CompiledConfig) {
    if let Some(maximum) = config.config.indent.maximum {
        let mut width = 0;
        for byte in unit.bytes().cycle().take(unit.len().saturating_mul(depth)) {
            width += if byte == b'\t' {
                config.config.indent.width
            } else {
                1
            };
            if width > maximum {
                break;
            }
            output.push(char::from(byte));
        }
    } else if !unit.is_empty() {
        for _ in 0..depth {
            output.push_str(unit);
        }
    }
}

fn render(
    source: &str,
    mut tokens: &[Token<'_>],
    opaque: &[Range<usize>],
    config: &CompiledConfig,
) -> String {
    let mut out = String::with_capacity(source.len());
    let unit = config.config.indent.unit("bst", "bst");
    let whitespace = &config.config.whitespace;
    let mut blanks = 0;
    let mut offset = 0;
    let default_ending = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for (number, line) in source.split_inclusive('\n').enumerate() {
        let (row, rest) = tokens.split_at(tokens.partition_point(|token| token.line == number));
        tokens = rest;
        let start = offset;
        offset += line.len();
        if crate::syntax::is_protected(start, opaque) {
            out.push_str(line);
            blanks = 0;
            continue;
        }
        let (body, ending) = split_line(line);
        if row.is_empty() {
            blanks += 1;
            if whitespace
                .blank_lines
                .limit()
                .is_none_or(|limit| blanks <= limit)
            {
                if !whitespace.remove_trailing {
                    out.push_str(body);
                }
                out.push_str(ending);
            }
            continue;
        }
        blanks = 0;
        let mut after = Gap::Tight;
        for (i, token) in row.iter().enumerate() {
            let comment = token.text.starts_with('%');
            if i == 0 || !comment && (after == Gap::Line || token.before == Gap::Line) {
                if i != 0 {
                    out.push_str(if ending.is_empty() {
                        default_ending
                    } else {
                        ending
                    });
                }
                write_indent(&mut out, &unit, token.depth, config);
            } else if comment || after != Gap::Tight && token.before != Gap::Tight {
                out.push(' ');
            }
            if comment && whitespace.remove_trailing {
                out.push_str(token.text.trim_end_matches([' ', '\t']));
            } else {
                out.push_str(token.text);
            }
            after = token.after;
        }
        if !whitespace.remove_trailing {
            let last = row.last().unwrap();
            out.push_str(&source[last.offset + last.text.len()..start + body.len()]);
        }
        out.push_str(ending);
    }
    out
}

pub(crate) fn format_style(
    source: &str,
    config: &CompiledConfig,
) -> Result<FormatOutput, FormatError> {
    let opaque = protected_ranges(source, false);
    // Read through valid disabled lines so braces crossing a directive still
    // determine depth. A disabled fragment may also contain non-BibTeX text.
    let parsed = parse(source, &[]).or_else(|err| {
        if opaque.is_empty() {
            Err(err)
        } else {
            parse(source, &opaque)
        }
    });
    let mut tokens = match parsed {
        Ok(tokens) => tokens,
        Err(diagnostic) => {
            return Ok(FormatOutput {
                text: source.into(),
                diagnostics: vec![diagnostic],
            });
        }
    };
    layout_functions(&mut tokens, config);
    let mut text = render(source, &tokens, &opaque, config);
    if config.config.comments.align_trailing {
        text = align_comments(&text, config)?;
    }
    Ok(FormatOutput {
        text,
        diagnostics: Vec::new(),
    })
}

fn align_comments(source: &str, config: &CompiledConfig) -> Result<String, FormatError> {
    let opaque = protected_ranges(source, false);
    let mut groups = Vec::new();
    let mut start = 0;
    for line in source.split_inclusive('\n') {
        let next = start + line.len();
        if !crate::syntax::is_protected(start, &opaque)
            && let Some((at, comment)) =
                lexemes(split_line(line).0).find(|(_, word)| word.starts_with('%'))
        {
            let code = line[..at].trim_end_matches([' ', '\t']);
            if !code.is_empty() && !comment.trim_start_matches('%').trim().is_empty() {
                let indent = &code[..code.len() - code.trim_start_matches([' ', '\t']).len()];
                let width = crate::wrap::display_width(code, 0, config.config.indent.width);
                groups.push((start, next, indent, start + code.len()..start + at, width));
            }
        }
        start = next;
    }
    crate::wrap::align_comment_groups(source, &groups)
}
