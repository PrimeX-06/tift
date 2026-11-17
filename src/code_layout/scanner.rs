//! Literal boundaries shared by the native code and docstrip printers.

use std::ops::Range;

use crate::{syntax, trivia};

pub(super) fn commands<'a>(
    source: &'a str,
    protected: &[Range<usize>],
) -> Vec<(usize, &'a str, usize)> {
    let mut output = Vec::new();
    let mut cursor = 0;
    let mut index = 0;
    while cursor < source.len() {
        while protected
            .get(index)
            .is_some_and(|range| range.end <= cursor)
        {
            index += 1;
        }
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end;
        } else if source.as_bytes()[cursor] == b'%' {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |end| cursor + end + 1);
        } else if let Some((name, end)) = syntax::command_at(source, cursor) {
            output.push((cursor, name, end));
            cursor = syntax::primitive_data_end(source, name, end).unwrap_or(end);
        } else {
            cursor += source[cursor..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
        }
    }
    output
}

pub(super) fn environment<'a>(source: &'a str, name: &str, end: usize) -> Option<(&'a str, usize)> {
    if !matches!(name, "begin" | "end") {
        return None;
    }
    let begin = trivia::skip(source, end);
    let finish = syntax::balanced_end(source, begin, b'{', b'}')?;
    Some((source[begin + 1..finish - 1].trim(), finish))
}

pub(super) fn environments(
    source: &str,
    protected: &[Range<usize>],
) -> Vec<(String, Range<usize>)> {
    let mut stack = Vec::new();
    let mut output = Vec::new();
    for (start, name, end) in commands(source, protected) {
        let Some((environment, finish)) = self::environment(source, name, end) else {
            continue;
        };
        if name == "begin" {
            stack.push((environment, start));
        } else if let Some(index) = stack.iter().rposition(|(open, _)| *open == environment) {
            let (_, begin) = stack.remove(index);
            output.push((environment.to_owned(), begin..finish));
        }
    }
    output
}

pub(super) fn expl3_regions(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut output = Vec::new();
    let mut open = None;
    for (start, name, mut end) in commands(source, protected) {
        if name == "ExplSyntaxOff" {
            if let Some(begin) = open.take() {
                output.push(begin..start);
            }
        } else if open.is_none() && (name == "ExplSyntaxOn" || name.starts_with("ProvidesExpl")) {
            if name != "ExplSyntaxOn" {
                for _ in 0..4 {
                    let begin = trivia::skip(source, end);
                    let Some(finish) = syntax::balanced_end(source, begin, b'{', b'}') else {
                        break;
                    };
                    end = finish;
                }
            }
            if source[end..].starts_with("\r\n") {
                end += 2;
            } else if source[end..].starts_with('\n') {
                end += 1;
            }
            open = Some(end);
        }
    }
    if let Some(begin) = open {
        output.push(begin..source.len());
    }
    output
}

pub(super) fn math_ranges(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for (name, range) in environments(source, protected) {
        if super::math_environment(&name) {
            ranges.push(range);
        }
    }
    let mut dollar = None;
    let mut cursor = 0;
    while cursor < source.len() {
        if syntax::is_protected(cursor, protected) {
            cursor = protected
                .iter()
                .find(|range| range.contains(&cursor))
                .expect("protected range")
                .end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |end| cursor + end + 1);
            continue;
        }
        if source.as_bytes()[cursor] == b'$' && !syntax::is_escaped(source, cursor) {
            let width = if source[cursor..].starts_with("$$") {
                2
            } else {
                1
            };
            if let Some((start, open_width)) = dollar {
                if width == open_width {
                    ranges.push(start..cursor + width);
                    dollar = None;
                }
            } else {
                dollar = Some((cursor, width));
            }
            cursor += width;
            continue;
        }
        if let Some((name, end)) = syntax::command_at(source, cursor) {
            if matches!(name, "[" | "(") {
                let close = if name == "[" { "\\]" } else { "\\)" };
                if let Some((start, _, finish)) = commands(&source[end..], &[])
                    .into_iter()
                    .find(|(start, _, _)| source[end + start..].starts_with(close))
                {
                    let _ = start;
                    ranges.push(cursor..end + finish);
                }
            } else if super::mathematical_command(name) {
                let shape = crate::packages::math::command(name)
                    .map(|signature| signature.shape)
                    .unwrap_or(match name {
                        "sqrt" | "smash" => "om",
                        "frac" | "dfrac" | "tfrac" | "binom" | "dbinom" | "tbinom" | "overset"
                        | "underset" | "stackrel" | "sideset" => "mm",
                        "prescript" => "mmm",
                        "mathchoice" => "mmmm",
                        _ => "m",
                    });
                let args = crate::layout::signature_argument_ranges(
                    source,
                    end..source.len(),
                    shape.bytes(),
                );
                if let Some(last) = args.last() {
                    ranges.push(cursor..last.end);
                }
            }
            cursor = end;
        } else {
            cursor += source[cursor..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
        }
    }
    super::merged(ranges)
}
