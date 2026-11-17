//! Bounded literal column-preamble parsing for source alignment.

use super::Justification;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

#[path = "column_spec/tabularray.rs"]
mod tabularray;

const MAX_COLUMNS: usize = 4096;
const MAX_DEPTH: usize = 32;

pub(super) const TABULARRAY_STYLE_COMMANDS: &[&str] = &[
    "SetCell",
    "SetCells",
    "SetRow",
    "SetRows",
    "SetColumn",
    "SetColumns",
];

#[derive(Default, Clone)]
pub(super) struct DeclaredTypes {
    columns: HashSet<u8>,
    literal_columns: HashMap<u8, LiteralColumn>,
    rows: HashSet<u8>,
    literal_rows: HashMap<u8, LiteralColumn>,
    selectors: HashSet<String>,
    tabularray_defaults: bool,
}

#[derive(Clone, Copy)]
struct LiteralColumn {
    arguments: usize,
    justification: Justification,
}

impl DeclaredTypes {
    fn for_rows(&self) -> Self {
        let mut result = self.clone();
        result.columns.clone_from(&self.rows);
        result.literal_columns.clone_from(&self.literal_rows);
        result
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct CellStyle {
    pub justification: Option<Justification>,
    pub colspan: Option<usize>,
    pub rowspan: Option<usize>,
}

impl CellStyle {
    fn overlay(&mut self, newer: Self) {
        self.justification = newer.justification.or(self.justification);
        self.colspan = newer.colspan.or(self.colspan);
        self.rowspan = newer.rowspan.or(self.rowspan);
    }
}

pub(super) struct TableSpec {
    columns: Vec<Justification>,
    ray: Option<tabularray::Spec>,
}

impl TableSpec {
    pub fn is_tabularray(&self) -> bool {
        self.ray.is_some()
    }

    pub fn column_count(&self) -> usize {
        self.ray
            .as_ref()
            .map_or(self.columns.len(), |spec| spec.column_count())
    }

    pub fn style(&self, row: usize, column: usize, rows: usize, columns: usize) -> CellStyle {
        self.ray.as_ref().map_or_else(
            || CellStyle {
                justification: self.columns.get(column - 1).copied(),
                ..CellStyle::default()
            },
            |spec| spec.style(row, column, rows, columns),
        )
    }

    pub fn body_styles(
        &self,
        source: &str,
        opaque: &[Range<usize>],
        cells: impl Iterator<Item = (usize, usize, Range<usize>)>,
    ) -> Result<tabularray::BodyStyles, ()> {
        tabularray::BodyStyles::parse(source, opaque, cells)
    }
}

pub(super) fn declared_types(source: &str, tokens: &[crate::syntax::Token]) -> DeclaredTypes {
    let mut declared = DeclaredTypes::default();
    for token in tokens {
        if token.kind != crate::syntax::TokenKind::Command {
            continue;
        }
        let Some((name, end)) = crate::syntax::command_at(source, token.span.start) else {
            continue;
        };
        if matches!(name, "SetTblrInner" | "SetTblrDefault") {
            declared.tabularray_defaults = true;
        }
        if let Some(body) = group(source, skip(source, end), b'{', b'}') {
            let letter = source[body.clone()].trim();
            if matches!(name, "NewTblrChildSelector" | "NewTblrChildIndexer") {
                declared.selectors.insert(letter.to_owned());
            } else if matches!(
                name,
                "newcolumntype"
                    | "renewcolumntype"
                    | "NewColumnType"
                    | "NewTblrColumnType"
                    | "NewTblrColumnRowType"
                    | "NewColumnRowType"
                    | "NewRowType"
                    | "NewTblrRowType"
            ) && let [byte] = letter.as_bytes()
            {
                // Complete local declarations take precedence over common
                // column defaults; opaque declarations keep the fallback.
                let literal = literal_column(source, body.end + 1);
                if !matches!(name, "NewRowType" | "NewTblrRowType") {
                    declared.columns.insert(*byte);
                    declared.literal_columns.remove(byte);
                    if let Some(column) = literal {
                        declared.literal_columns.insert(*byte, column);
                    }
                }
                if matches!(
                    name,
                    "NewRowType" | "NewTblrRowType" | "NewTblrColumnRowType" | "NewColumnRowType"
                ) {
                    declared.rows.insert(*byte);
                    declared.literal_rows.remove(byte);
                    if let Some(row) = literal {
                        declared.literal_rows.insert(*byte, row);
                    }
                }
            }
        }
    }
    declared
}

pub(super) fn for_environment(
    name: &str,
    header: &str,
    declared: &DeclaredTypes,
) -> Option<TableSpec> {
    if matches!(name, "tabu" | "longtabu") {
        let range = crate::package_tables::tabu_column_spec(header, 0..header.len())?;
        return Some(TableSpec {
            columns: parse(&header[range.start + 1..range.end - 1], declared)?,
            ray: None,
        });
    }
    let signature = crate::packages::header_signature(name)?;
    let slot = crate::packages::column_spec_slot(name)?;
    let mut cursor = 0;
    let mut selected = None;
    for (index, kind) in signature.bytes().enumerate() {
        let before_space = cursor;
        cursor = skip(header, cursor);
        match kind {
            b'o' | b'O' => {
                if header.as_bytes().get(cursor) == Some(&b'[')
                    && (kind == b'o' || before_space == cursor)
                {
                    let body = group(header, cursor, b'[', b']')?;
                    cursor = body.end + 1;
                    if index == slot {
                        selected = Some(&header[body]);
                    }
                }
            }
            b'm' => {
                let body = group(header, cursor, b'{', b'}')?;
                cursor = body.end + 1;
                if index == slot {
                    selected = Some(&header[body]);
                }
            }
            _ => return None,
        }
    }
    if crate::package_tables::is_tabularray(name) {
        if declared.tabularray_defaults {
            return None;
        }
        let default = if matches!(
            name,
            "+matrix" | "+bmatrix" | "+Bmatrix" | "+pmatrix" | "+vmatrix" | "+Vmatrix"
        ) {
            Justification::Center
        } else {
            Justification::Left
        };
        let ray = tabularray::Spec::parse(selected.unwrap_or(""), default, declared).ok()?;
        Some(TableSpec {
            columns: Vec::new(),
            ray: Some(ray),
        })
    } else {
        Some(TableSpec {
            columns: parse(selected?, declared)?,
            ray: None,
        })
    }
}

pub(super) fn multicolumn(
    text: &str,
    declared: &DeclaredTypes,
    _environment: Option<&str>,
) -> Option<(usize, Justification)> {
    let (command, mut cursor) = crate::syntax::command_at(text, 0)?;
    if command != "multicolumn" {
        return None;
    }
    cursor = skip(text, cursor);
    let span = group(text, cursor, b'{', b'}')?;
    let count = text[span.clone()].trim().parse::<usize>().ok()?;
    if !(1..=256).contains(&count) {
        return None;
    }
    cursor = skip(text, span.end + 1);
    let spec = group(text, cursor, b'{', b'}')?;
    let parsed = parse(&text[spec], declared)?;
    (parsed.len() == 1).then_some((count, parsed[0]))
}

pub(super) fn parse(source: &str, declared: &DeclaredTypes) -> Option<Vec<Justification>> {
    let mut columns = Vec::new();
    parse_into(source, &mut columns, 0, declared)?;
    (!columns.is_empty()).then_some(columns)
}

fn parse_into(
    source: &str,
    output: &mut Vec<Justification>,
    depth: usize,
    declared: &DeclaredTypes,
) -> Option<()> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let mut cursor = 0;
    while cursor < source.len() {
        cursor = skip(source, cursor);
        let Some(&byte) = source.as_bytes().get(cursor) else {
            break;
        };
        cursor += 1;
        if let Some(column) = declared.literal_columns.get(&byte) {
            for _ in 0..column.arguments {
                cursor = skip(source, cursor);
                cursor = group(source, cursor, b'{', b'}')?.end + 1;
            }
            output.push(column.justification);
            if output.len() > MAX_COLUMNS {
                return None;
            }
            continue;
        }
        if declared.columns.contains(&byte) {
            return None;
        }
        let align = match byte {
            b'l' => Justification::Left,
            b'c' => Justification::Center,
            b'r' => Justification::Right,
            b'p' | b'm' | b'b' => {
                cursor = skip(source, cursor);
                let mut align = Justification::Left;
                if source.as_bytes().get(cursor) == Some(&b'[') {
                    let options = group(source, cursor, b'[', b']')?;
                    align = option_alignment(&source[options.clone()])
                        .ok()?
                        .unwrap_or(align);
                    cursor = skip(source, options.end + 1);
                }
                cursor = group(source, cursor, b'{', b'}')?.end + 1;
                align
            }
            b'X' | b'Q' | b'L' | b'C' | b'R' => {
                let align = match byte {
                    b'C' => Justification::Center,
                    b'R' => Justification::Right,
                    _ => Justification::Left,
                };
                let after = skip(source, cursor);
                let align = if source.as_bytes().get(after) == Some(&b'[') {
                    let options = group(source, after, b'[', b']')?;
                    cursor = options.end + 1;
                    option_alignment(&source[options]).ok()?.unwrap_or(align)
                } else {
                    align
                };
                if matches!(byte, b'L' | b'C' | b'R') {
                    let after = skip(source, cursor);
                    if source.as_bytes().get(after) == Some(&b'{') {
                        cursor = group(source, after, b'{', b'}')?.end + 1;
                    }
                }
                align
            }
            b'S' | b's' => {
                let after = skip(source, cursor);
                if source.as_bytes().get(after) == Some(&b'[') {
                    cursor = group(source, after, b'[', b']')?.end + 1;
                }
                Justification::Center
            }
            b'|' | b':' => continue,
            b'>' | b'<' | b'@' | b'!' => {
                cursor = skip(source, cursor);
                cursor = group(source, cursor, b'{', b'}')?.end + 1;
                continue;
            }
            b'*' => {
                cursor = skip(source, cursor);
                let count = group(source, cursor, b'{', b'}')?;
                let count_value = source[count.clone()].trim().parse::<usize>().ok()?;
                cursor = skip(source, count.end + 1);
                let body = group(source, cursor, b'{', b'}')?;
                let mut repeated = Vec::new();
                parse_into(&source[body.clone()], &mut repeated, depth + 1, declared)?;
                let additional = repeated.len().checked_mul(count_value)?;
                if output.len().checked_add(additional)? > MAX_COLUMNS {
                    return None;
                }
                if !repeated.is_empty() {
                    for _ in 0..count_value {
                        output.extend_from_slice(&repeated);
                    }
                }
                cursor = body.end + 1;
                continue;
            }
            // Custom column letters can consume arbitrary arguments. Their
            // arity cannot be inferred from the letter alone.
            _ => return None,
        };
        // The declared column type remains authoritative; modifier bodies
        // above are retained without evaluating arbitrary TeX commands.
        output.push(align);
        if output.len() > MAX_COLUMNS {
            return None;
        }
    }
    Some(())
}

/// Recognize a single literal column alias and its mandatory argument count.
/// Width expressions remain opaque; declaration bodies are never executed.
fn literal_column(source: &str, start: usize) -> Option<LiteralColumn> {
    let mut cursor = skip(source, start);
    let mut arguments = 0;
    if source.as_bytes().get(cursor) == Some(&b'[') {
        let count = group(source, cursor, b'[', b']')?;
        arguments = source[count.clone()].trim().parse::<usize>().ok()?;
        if arguments > 9 {
            return None;
        }
        cursor = skip(source, count.end + 1);
        // Optional custom arguments require a distinct package-specific model.
        if source.as_bytes().get(cursor) == Some(&b'[') {
            return None;
        }
    }
    let body = group(source, cursor, b'{', b'}')?;
    let body = &source[body];
    let mut cursor = skip(body, 0);
    let mut modifier = None;
    while body.as_bytes().get(cursor) == Some(&b'>') {
        cursor = skip(body, cursor + 1);
        let content = group(body, cursor, b'{', b'}')?;
        let mut at = skip(body, content.start);
        while at < content.end {
            let (command, end) = crate::syntax::command_at(body, at)?;
            modifier = match command {
                "raggedright" | "RaggedRight" => Some(Justification::Left),
                "centering" | "Centering" => Some(Justification::Center),
                "raggedleft" | "RaggedLeft" => Some(Justification::Right),
                "arraybackslash" => modifier,
                _ => return None,
            };
            at = skip(body, end);
        }
        cursor = skip(body, content.end + 1);
    }
    let columns = parse(&body[cursor..], &DeclaredTypes::default())?;
    let [justification] = columns.as_slice() else {
        return None;
    };
    Some(LiteralColumn {
        arguments,
        justification: modifier.unwrap_or(*justification),
    })
}

fn option_alignment(source: &str) -> Result<Option<Justification>, ()> {
    let mut result = None;
    for option in parts(source).ok_or(())? {
        let option = option.trim();
        let (value, required) = match option.split_once('=') {
            Some((key, value)) if key.trim() == "halign" => (value.trim(), true),
            Some(_) => continue,
            None if option.contains('\\') => return Err(()),
            None => (option, false),
        };
        let value = if value.starts_with('{') {
            let body = group(value, 0, b'{', b'}').ok_or(())?;
            if skip(value, body.end + 1) != value.len() {
                return Err(());
            }
            value[body].trim()
        } else {
            value
        };
        result = match value {
            "l" | "j" => Some(Justification::Left),
            "c" => Some(Justification::Center),
            "r" => Some(Justification::Right),
            _ if required => return Err(()),
            _ => result,
        };
    }
    Ok(result)
}

fn parts(source: &str) -> Option<Vec<&str>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'{' | b'[' => {
                let open = source.as_bytes()[cursor];
                cursor =
                    group(source, cursor, open, if open == b'{' { b'}' } else { b']' })?.end + 1;
            }
            b',' => {
                result.push(&source[start..cursor]);
                cursor += 1;
                start = cursor;
            }
            b'%' => cursor = comment_end(source, cursor),
            b'\\' => {
                cursor = crate::syntax::command_at(source, cursor)?.1;
            }
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    result.push(&source[start..]);
    Some(result)
}

fn skip(source: &str, mut cursor: usize) -> usize {
    while cursor < source.len() {
        if source.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
        } else if source.as_bytes()[cursor] == b'%' {
            cursor = comment_end(source, cursor);
        } else {
            break;
        }
    }
    cursor
}

fn comment_end(source: &str, cursor: usize) -> usize {
    source[cursor..]
        .find('\n')
        .map_or(source.len(), |n| cursor + n + 1)
}

fn group(source: &str, start: usize, open: u8, close: u8) -> Option<Range<usize>> {
    if source.as_bytes().get(start) != Some(&open) {
        return None;
    }
    let mut depth = 1usize;
    let mut cursor = start + 1;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'\\' => cursor = crate::syntax::command_at(source, cursor)?.1,
            b'%' => cursor = comment_end(source, cursor),
            byte if byte == open => {
                depth += 1;
                cursor += 1;
            }
            byte if byte == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + 1..cursor);
                }
                cursor += 1;
            }
            b'{' if open == b'[' => cursor = group(source, cursor, b'{', b'}')?.end + 1,
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    None
}


