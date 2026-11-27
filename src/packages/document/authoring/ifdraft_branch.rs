use std::ops::Range;

use crate::{syntax, trivia};

#[derive(Debug, Clone)]
pub(crate) struct Ifdraft {
    draft: bool,
    class_option: Option<bool>,
    checked: usize,
}

impl Ifdraft {
    pub(crate) fn load(prefix: &str, option: &str, checked: usize) -> Option<Self> {
        let draft = match option.trim_matches([' ', '\t', '\r', '\n']) {
            "draft" => true,
            "final" => false,
            _ => return None,
        };
        let mut cursor = trivia::skip(prefix, 0);
        let mut class_option = None;
        if cursor < prefix.len() {
            let ("documentclass", end) = syntax::command_at(prefix, cursor)? else {
                return None;
            };
            cursor = trivia::skip(prefix, end);
            if prefix.as_bytes().get(cursor) == Some(&b'[') {
                let end = syntax::balanced_end(prefix, cursor, b'[', b']')?;
                for option in prefix[cursor + 1..end - 1].split(',').map(str::trim_ascii) {
                    match option {
                        "draft" | "final" if class_option.is_none() => {
                            class_option = Some(option == "draft");
                        }
                        "10pt" | "11pt" | "12pt" | "a4paper" | "letterpaper" | "oneside"
                        | "twoside" => (),
                        _ => return None,
                    }
                }
                cursor = trivia::skip(prefix, end);
            }
            let end = syntax::balanced_end(prefix, cursor, b'{', b'}')?;
            if !matches!(&prefix[cursor + 1..end - 1], "article" | "book" | "report")
                || trivia::skip(prefix, end) != prefix.len()
            {
                return None;
            }
        }
        (!prefix.contains("^^")).then_some(Self {
            draft,
            class_option,
            checked,
        })
    }

    pub(crate) fn branch(
        &mut self,
        source: &str,
        name: &str,
        command: Range<usize>,
    ) -> Option<(Range<usize>, usize)> {
        let first = match name {
            "ifdraft" => self.draft,
            "ifoptiondraft" => self.draft || self.class_option == Some(true),
            "ifoptionfinal" => !self.draft || self.class_option == Some(false),
            _ => return None,
        };
        plain_segment(source.get(self.checked..command.start)?)?;
        let mut cursor = command.end;
        let left = operand(source, &mut cursor)?;
        let right = operand(source, &mut cursor)?;
        let selected = if first { left } else { right };
        let body = if source.as_bytes()[selected.start] == b'{' {
            selected.start + 1..selected.end - 1
        } else if source.as_bytes()[selected.start].is_ascii_alphanumeric() {
            selected
        } else {
            return None;
        };
        plain_segment(&source[body.clone()])?;
        self.checked = cursor;
        Some((body, cursor))
    }
}

fn plain_segment(text: &str) -> Option<()> {
    if text.contains("^^") {
        return None;
    }
    let mut cursor = trivia::skip(text, 0);
    while cursor < text.len() {
        let byte = text.as_bytes()[cursor];
        cursor = if byte == b'\\' {
            let (name, end) = syntax::command_at(text, cursor)?;
            match name {
                "relax" | "begingroup" | "endgroup" => end,
                "begin" | "end" => {
                    let at = trivia::skip(text, end);
                    let finish = syntax::balanced_end(text, at, b'{', b'}')?;
                    if !matches!(
                        &text[at + 1..finish - 1],
                        "document" | "quote" | "quotation" | "equation"
                    ) {
                        return None;
                    }
                    finish
                }
                "def" => simple_definition_end(text, end)?,
                _ => return None,
            }
        } else if byte.is_ascii_alphanumeric() || b".,;:!?()[]{}$+-=*/_".contains(&byte) {
            cursor + 1
        } else {
            return None;
        };
        cursor = trivia::skip(text, cursor);
    }
    Some(())
}

fn simple_definition_end(text: &str, end: usize) -> Option<usize> {
    let (target, after) = syntax::command_at(text, trivia::skip(text, end))?;
    if !target.bytes().all(|byte| byte.is_ascii_alphabetic())
        || target.starts_with("end")
        || target.starts_with("if")
        || syntax::context_builtin_command(target)
        || matches!(
            target,
            "expandafter"
                | "else"
                | "fi"
                | "def"
                | "relax"
                | "begin"
                | "begingroup"
                | "document"
                | "quote"
                | "quotation"
                | "equation"
        )
    {
        return None;
    }
    let brace = syntax::parameter_text_end(text, after)?;
    syntax::arguments::undelimited_signature(&text[after..brace])?;
    let finish = syntax::balanced_end(text, brace, b'{', b'}')?;
    text[brace + 1..finish - 1]
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'#')
        .then_some(finish)
}

fn operand(source: &str, cursor: &mut usize) -> Option<Range<usize>> {
    let previous = *cursor;
    let at = trivia::skip(source, *cursor);
    let gap = &source[*cursor..at];
    if gap
        .split_inclusive('\n')
        .skip(1)
        .any(|line| line.ends_with('\n') && line.trim_ascii().is_empty())
    {
        return None;
    }
    let byte = *source.as_bytes().get(at)?;
    *cursor = match byte {
        b'{' => syntax::balanced_end(source, at, b'{', b'}')?,
        b'\\' => {
            let (name, end) = syntax::command_at(source, at)?;
            name.bytes()
                .all(|byte| byte.is_ascii_alphabetic())
                .then_some(end)?
        }
        b'}' | b'^' | b'#' | b'%' => return None,
        byte if byte.is_ascii_graphic() => at + 1,
        _ => return None,
    };
    (!source[previous..*cursor].contains(['\u{b}', '\u{c}'])
        && !source[previous..*cursor].contains("^^"))
    .then_some(at..*cursor)
}
