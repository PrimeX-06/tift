//! Bounded printers for braced algorithm statements and expl3 calls.

use crate::{syntax, trivia};

const MAX_DEPTH: usize = 256;

fn compact(source: &str) -> String {
    let mut result = String::new();
    let mut cursor = 0;
    let mut space = false;
    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'%' {
            let end = source[cursor..]
                .find('\n')
                .map_or(source.len(), |offset| cursor + offset + 1);
            if space && !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&source[cursor..end]);
            cursor = end;
            space = false;
        } else if let Some((_, end)) = syntax::command_at(source, cursor) {
            if space && !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&source[cursor..end]);
            cursor = end;
            space = false;
        } else {
            let character = source[cursor..].chars().next().expect("source character");
            cursor += character.len_utf8();
            if character.is_ascii_whitespace() {
                space = true;
            } else {
                if space && !result.is_empty() && character != '}' && !result.ends_with('{') {
                    result.push(' ');
                }
                result.push(character);
                space = false;
            }
        }
    }
    result
}

struct Printer {
    output: String,
    width: usize,
    level: usize,
}

impl Printer {
    fn append(&mut self, source: &str) {
        if source.is_empty() {
            return;
        }
        if self.output.is_empty() || self.output.ends_with('\n') {
            self.output.push_str(&" ".repeat(self.level * self.width));
        }
        self.output.push_str(source);
    }

    fn newline(&mut self) {
        while self.output.ends_with([' ', '\t']) {
            self.output.pop();
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn group(&mut self, body: &str, expl3: bool, statements: bool, depth: usize) {
        self.append("{");
        self.newline();
        self.level += 1;
        self.body(body, expl3, statements, depth + 1);
        self.newline();
        self.level -= 1;
        self.append("}");
    }

    fn keys_group(&mut self, source: &str) {
        self.append("{");
        self.newline();
        self.level += 1;
        for entry in crate::packages::keys::definition_entries(source) {
            let text = compact(&source[entry]);
            for line in text.split_inclusive('\n') {
                self.append(line);
            }
            self.newline();
        }
        self.level -= 1;
        self.append("}");
    }

    fn body(&mut self, source: &str, expl3: bool, statements: bool, depth: usize) {
        if depth >= MAX_DEPTH {
            self.append(source);
            return;
        }
        let mut cursor = 0;
        let mut text = 0;
        while cursor < source.len() {
            if source.as_bytes()[cursor] == b'%' {
                self.append(&compact(&source[text..cursor]));
                if !self.output.ends_with(['\n', ' ']) {
                    self.append(" ");
                }
                let end = source[cursor..]
                    .find('\n')
                    .map_or(source.len(), |offset| cursor + offset);
                self.append(&source[cursor..end]);
                self.newline();
                cursor = end + usize::from(end < source.len());
                text = cursor;
                continue;
            }
            let Some((name, end)) = syntax::command_at(source, cursor) else {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("source character")
                    .len_utf8();
                continue;
            };
            if name == "begin"
                && let Some((environment, header)) = super::scanner::environment(source, name, end)
                && super::algorithm_environment(environment)
                && let Some((_, range)) = super::scanner::environments(&source[cursor..], &[])
                    .into_iter()
                    .find(|(candidate, range)| candidate == environment && range.start == 0)
            {
                self.append(&compact(&source[text..cursor]));
                self.newline();
                self.append(&compact(&source[cursor..header]));
                self.newline();
                self.level += 1;
                let fragment_end = cursor + range.end;
                let closing = super::scanner::commands(&source[header..fragment_end], &[])
                    .into_iter()
                    .rev()
                    .find(|(_, command, _)| *command == "end")
                    .map(|(offset, _, _)| header + offset)
                    .expect("algorithm closing");
                self.body(&source[header..closing], false, false, depth + 1);
                self.newline();
                self.level -= 1;
                self.append(&source[closing..fragment_end]);
                self.newline();
                cursor = fragment_end;
                text = cursor;
                continue;
            }
            let shape = match name.trim_end_matches('*') {
                "For" | "ForEach" | "ForAll" | "While" | "If" | "ElseIf" | "Repeat" => Some((2, 1)),
                "eIf" => Some((3, 1)),
                "Else" => Some((1, 0)),
                _ => None,
            };
            if !expl3 && let Some((count, first_body)) = shape {
                let arguments = crate::layout::signature_argument_ranges(
                    source,
                    end..source.len(),
                    std::iter::repeat_n(b'm', count),
                );
                if arguments.len() == count {
                    self.append(&compact(&source[text..cursor]));
                    self.newline();
                    self.append(&source[cursor..end]);
                    for (index, argument) in arguments.iter().enumerate() {
                        if index < first_body {
                            self.append(&compact(&source[argument.clone()]));
                        } else {
                            self.group(
                                &source[argument.start + 1..argument.end - 1],
                                false,
                                true,
                                depth,
                            );
                        }
                    }
                    self.newline();
                    cursor = arguments.last().expect("control arguments").end;
                    text = cursor;
                    continue;
                }
            }
            if expl3 && name.contains(':') {
                self.append(&compact(&source[text..cursor]));
                self.newline();
                self.append(&source[cursor..end]);
                let suffix = name.rsplit_once(':').expect("expl3 name").1;
                let definition = name.starts_with("cs_new")
                    || name.starts_with("cs_set")
                    || name.starts_with("cs_gset");
                let mut finish = end;
                for (slot, kind) in suffix.chars().enumerate() {
                    let begin = trivia::skip(source, finish);
                    if kind == 'p' {
                        let mut scan = begin;
                        while scan < source.len() && source.as_bytes()[scan] != b'{' {
                            scan = syntax::command_at(source, scan).map_or_else(
                                || {
                                    scan + source[scan..]
                                        .chars()
                                        .next()
                                        .expect("source character")
                                        .len_utf8()
                                },
                                |(_, end)| end,
                            );
                        }
                        let parameters = compact(&source[begin..scan]);
                        if !parameters.is_empty() {
                            self.append(" ");
                            self.append(&parameters);
                        }
                        finish = scan;
                        continue;
                    }
                    if source.as_bytes().get(begin) == Some(&b'{') {
                        let Some(close) = syntax::balanced_end(source, begin, b'{', b'}') else {
                            break;
                        };
                        let body = &source[begin + 1..close - 1];
                        if name == "keys_define:nn" && slot == 1 {
                            self.newline();
                            self.level += 1;
                            self.keys_group(body);
                            self.level -= 1;
                        } else if definition && slot + 1 == suffix.len() {
                            self.newline();
                            self.level += 1;
                            self.group(body, true, false, depth);
                            self.level -= 1;
                        } else {
                            self.append(" ");
                            self.append(&compact(&source[begin..close]));
                        }
                        finish = close;
                    } else if let Some((_, close)) = syntax::command_at(source, begin) {
                        self.append(" ");
                        self.append(&source[begin..close]);
                        finish = close;
                    } else {
                        break;
                    }
                }
                // Keep a source-line comment attached to its statement. Moving
                // it to column zero would turn code comments into DTX prose on
                // the next pass and change their prefix again.
                let mut comment = finish;
                while matches!(source.as_bytes().get(comment), Some(b' ' | b'\t')) {
                    comment += 1;
                }
                if source.as_bytes().get(comment) == Some(&b'%') {
                    let end = source[comment..]
                        .find('\n')
                        .map_or(source.len(), |n| comment + n);
                    self.append(" ");
                    self.append(&source[comment..end]);
                    finish = end + usize::from(end < source.len());
                }
                self.newline();
                cursor = finish;
                text = cursor;
                continue;
            }
            if name == ";" && statements {
                self.append(&compact(&source[text..end]));
                let next = end
                    + source[end..]
                        .bytes()
                        .take_while(|byte| matches!(byte, b' ' | b'\t'))
                        .count();
                if source.as_bytes().get(next) != Some(&b'%') {
                    self.newline();
                }
                cursor = end;
                text = end;
                continue;
            }
            cursor = end;
        }
        self.append(&compact(&source[text..]));
    }
}

pub(super) fn format(source: &str, expl3: bool, width: usize) -> String {
    let mut printer = Printer {
        output: String::new(),
        width,
        level: 0,
    };
    printer.body(source, expl3, false, 0);
    printer.newline();
    printer.output
}
