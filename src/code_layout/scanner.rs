//! Literal boundaries shared by the native code and docstrip printers.

use std::collections::HashMap;
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
            cursor = syntax::primitive_data_end(source, name, end)
                .or_else(|| syntax::let_declaration_end(source, name, end))
                .unwrap_or(end);
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

pub(super) fn algorithm_header_end(source: &str, end: usize) -> usize {
    let start = trivia::skip(source, end);
    syntax::balanced_end(source, start, b'[', b']').unwrap_or(end)
}

const ENVIRONMENT_INDEX_DEPTH: usize = 64;

pub(super) fn environments<'a>(
    source: &'a str,
    protected: &[Range<usize>],
) -> Vec<(&'a str, Range<usize>)> {
    let mut stack = Vec::<(&str, usize)>::new();
    let mut indexed_stack = None::<HashMap<&str, Vec<usize>>>;
    let mut output = Vec::new();
    let mut header_end = 0;
    for (start, name, end) in commands(source, protected) {
        if start < header_end {
            continue;
        }
        let Some((environment, finish)) = self::environment(source, name, end) else {
            continue;
        };
        if name == "begin" {
            if super::algorithm_environment(environment) {
                header_end = algorithm_header_end(source, finish);
            }
            if let Some(index) = &mut indexed_stack {
                index.entry(environment).or_default().push(start);
            } else {
                stack.push((environment, start));
                if stack.len() > ENVIRONMENT_INDEX_DEPTH {
                    let mut index = HashMap::<&str, Vec<usize>>::new();
                    for (open, begin) in stack.drain(..) {
                        index.entry(open).or_default().push(begin);
                    }
                    indexed_stack = Some(index);
                }
            }
        } else if let Some(index) = &mut indexed_stack {
            if let Some(begin) = index.get_mut(environment).and_then(Vec::pop) {
                output.push((environment, begin..finish));
            }
        } else if stack.last().is_some_and(|(open, _)| *open == environment) {
            let (_, begin) = stack.pop().expect("matching environment on stack");
            output.push((environment, begin..finish));
        } else if let Some(index) = stack.iter().rposition(|(open, _)| *open == environment) {
            let (_, begin) = stack.remove(index);
            output.push((environment, begin..finish));
        }
    }
    output
}

pub(super) fn expl3_regions(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut output = Vec::new();
    let mut open = None;
    let mut delayed_end = 0;
    for (start, name, mut end) in commands(source, protected) {
        if start < delayed_end {
            continue;
        }
        if let Some(finish) = syntax::delayed_data_end(source, name, end).or_else(|| {
            crate::source_model::expl3_stored_arguments(source, name, end)
                .map(|argument| argument.end)
        }) {
            delayed_end = finish;
            continue;
        }
        if name == "ExplSyntaxOff" {
            if let Some(begin) = open.take() {
                output.push(begin..start);
            }
        } else if open.is_none()
            && matches!(
                name,
                "ExplSyntaxOn" | "ProvidesExplPackage" | "ProvidesExplClass" | "ProvidesExplFile"
            )
        {
            if name != "ExplSyntaxOn" {
                for _ in 0..4 {
                    let begin = trivia::skip(source, end);
                    let Some(finish) = syntax::balanced_end(source, begin, b'{', b'}') else {
                        break;
                    };
                    end = finish;
                }
            }
            // Leading horizontal trivia separates the opening control word
            // from letter tokens when the first code is on the same line.
            while matches!(source.as_bytes().get(end), Some(b' ' | b'\t')) {
                end += 1;
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
        if super::math_environment(name) {
            ranges.push(range);
        }
    }
    let mut dollar = None;
    let mut cursor = 0;
    let mut protection_index = 0;
    let mut absent_closers = [false; 2];
    while cursor < source.len() {
        while protected
            .get(protection_index)
            .is_some_and(|range| range.end <= cursor)
        {
            protection_index += 1;
        }
        if let Some(range) = protected
            .get(protection_index)
            .filter(|range| range.start <= cursor)
        {
            cursor = range.end;
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
                let close = if name == "[" { "]" } else { ")" };
                let slot = usize::from(name == "[");
                if !absent_closers[slot] {
                    if let Some(finish) = find_math_close(source, end, close, protected) {
                        ranges.push(cursor..finish);
                        cursor = finish;
                        continue;
                    }
                    absent_closers[slot] = true;
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
                    cursor = last.end;
                    continue;
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

fn find_math_close(
    source: &str,
    mut cursor: usize,
    close: &str,
    protected: &[Range<usize>],
) -> Option<usize> {
    let mut index = protected.partition_point(|range| range.end <= cursor);
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
            if name == close {
                return Some(end);
            }
            cursor = syntax::primitive_data_end(source, name, end).unwrap_or(end);
        } else {
            cursor += source[cursor..].chars().next()?.len_utf8();
        }
    }
    None
}
