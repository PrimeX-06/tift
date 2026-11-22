//! Bounded printers for braced algorithm statements and expl3 calls.

use crate::syntax;

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
            if space && !result.is_empty() && !result.ends_with('\n') {
                result.push(' ');
            }
            result.push_str(&source[cursor..end]);
            cursor = end;
            space = false;
        } else if let Some((_, end)) = syntax::command_at(source, cursor) {
            if space && !result.is_empty() && !result.ends_with('\n') {
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
                if space && !result.is_empty() && character != '}' && !result.ends_with(['{', '\n'])
                {
                    result.push(' ');
                }
                result.push(character);
                space = false;
            }
        }
    }
    result
}

struct Printer<'a> {
    output: String,
    width: usize,
    level: usize,
    expl3_scope_depth: usize,
    marker_prefix: &'a str,
}

impl Printer<'_> {
    // Unrecognized TeX commands and opaque readers can depend on a physical
    // line ending (for example, \endinput or a verbatim environment's end).
    // Keep their existing line boundaries while formatting the surrounding
    // expl3 statements. Argument compaction remains a separate operation.
    fn text(&mut self, source: &str, expl3: bool) {
        if expl3 {
            for line in source.split_inclusive('\n') {
                self.append(&compact(line));
                if line.ends_with('\n') {
                    self.newline();
                }
            }
        } else {
            self.append(&compact(source));
        }
    }

    // Trivia between actual arguments may contain comments. Keep their bytes
    // and the distinction between an inline comment and a standalone one.
    fn argument_comments(&mut self, gap: &str) {
        if let Some(comment) = gap.find('%') {
            if gap[..comment].contains('\n') {
                self.newline();
            } else if !self.output.ends_with([' ', '\n']) {
                self.append(" ");
            }
            let tail = gap[comment..].trim_end_matches([' ', '\t']);
            self.append(tail);
        }
    }

    fn argument_space(&mut self) {
        while self.output.ends_with([' ', '\t']) {
            self.output.pop();
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.append(" ");
        }
    }

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
                self.text(&source[text..cursor], expl3);
                if !self.output.is_empty() && !self.output.ends_with(['\n', ' ']) {
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
            if name
                .strip_prefix(self.marker_prefix)
                .is_some_and(|suffix| suffix.starts_with("DISPLAY"))
            {
                self.text(&source[text..cursor], expl3);
                self.newline();
                let end = end + usize::from(source[end..].starts_with("{}")) * 2;
                self.append(&source[cursor..end]);
                self.newline();
                cursor = end;
                text = end;
                continue;
            }
            if name == "begin"
                && let Some((environment, header)) = super::scanner::environment(source, name, end)
                && super::algorithm_environment(environment)
                && let Some((_, range)) = super::scanner::environments(&source[cursor..], &[])
                    .into_iter()
                    .find(|(candidate, range)| *candidate == environment && range.start == 0)
                && let fragment_end = cursor + range.end
                && let header = super::scanner::algorithm_header_end(source, header)
                && let Some(body_and_close) = source.get(header..fragment_end)
                && let Some((offset, _, _)) = super::scanner::commands(body_and_close, &[])
                    .into_iter()
                    .rev()
                    .find(|&(_, command, end)| {
                        command == "end"
                            && super::scanner::environment(body_and_close, command, end)
                                == Some((environment, body_and_close.len()))
                    })
            {
                let closing = header + offset;
                self.text(&source[text..cursor], expl3);
                self.newline();
                self.append(&compact(&source[cursor..header]));
                self.newline();
                self.level += 1;
                self.body(&source[header..closing], false, false, depth + 1);
                self.newline();
                self.level -= 1;
                self.append(&source[closing..fragment_end]);
                self.newline();
                cursor = fragment_end;
                text = cursor;
                continue;
            }
            if expl3 && name == "group_begin:" {
                self.text(&source[text..cursor], true);
                self.newline();
                self.append(&source[cursor..end]);
                self.newline();
                self.level += 1;
                self.expl3_scope_depth += 1;
                cursor = end;
                text = cursor;
                continue;
            }
            if expl3 && name == "group_end:" {
                self.text(&source[text..cursor], true);
                if self.expl3_scope_depth > 0 {
                    self.level = self.level.saturating_sub(1);
                    self.expl3_scope_depth -= 1;
                }
                self.newline();
                self.append(&source[cursor..end]);
                self.newline();
                cursor = end;
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
                    self.text(&source[text..cursor], expl3);
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
            if expl3 && super::expl3::is_expl3_name(name) {
                let Some(call) = super::expl3::parse_call(source, name, end) else {
                    self.text(&source[text..cursor], true);
                    self.append(&source[cursor..]);
                    return;
                };
                self.text(&source[text..cursor], expl3);
                self.newline();
                self.append(&source[cursor..end]);
                let mut finish = end;
                for (slot, argument) in call.arguments.iter().enumerate() {
                    let range = &argument.range;
                    self.argument_comments(&source[finish..range.start]);
                    if argument.kind == super::expl3::ArgumentKind::ParameterText {
                        let parameters = &source[range.clone()];
                        if !parameters.is_empty() {
                            self.argument_space();
                            if parameters.contains('%') {
                                self.append(parameters.trim_end_matches([' ', '\t']));
                            } else {
                                self.append(&compact(parameters));
                            }
                        }
                    } else if source.as_bytes().get(range.start) == Some(&b'{') {
                        let body = &source[range.start + 1..range.end - 1];
                        let kind = call.signature.as_bytes()[slot];
                        if call.base == "keys_define" && slot == 1 {
                            self.newline();
                            self.level += 1;
                            self.keys_group(body);
                            self.level -= 1;
                        } else if super::expl3::is_code_argument(&call, slot, kind) {
                            self.newline();
                            self.level += 1;
                            self.group(body, true, false, depth);
                            self.level -= 1;
                        } else {
                            self.argument_space();
                            self.append(&compact(&source[range.clone()]));
                        }
                    } else {
                        self.argument_space();
                        self.append(&source[range.clone()]);
                    }
                    finish = range.end;
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
                cursor = call.end.max(finish);
                text = cursor;
                continue;
            }
            if name == ";" && statements {
                self.text(&source[text..end], expl3);
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
        self.text(&source[text..], expl3);
    }
}

pub(super) fn format(opaque: &super::Opaque, expl3: bool, width: usize) -> String {
    let mut printer = Printer {
        output: String::new(),
        width,
        level: 0,
        expl3_scope_depth: 0,
        marker_prefix: opaque.marker_prefix.trim_start_matches('\\'),
    };
    printer.body(opaque.source(), expl3, false, 0);
    printer.newline();
    printer.output
}
