//! Bounded literal column-preamble parsing for source alignment.

use super::Justification;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

#[path = "column_spec/tabularray.rs"]
mod tabularray;

const MAX_COLUMNS: usize = 4096;
const MAX_DEPTH: usize = 32;

pub(crate) fn format_preambles(
    source: &str,
    config: &crate::CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, crate::FormatError> {
    use super::ColumnSpecLayout;
    use crate::syntax;
    use unicode_width::UnicodeWidthStr;
    if !source.contains("\\begin") {
        return Ok(source.into());
    }
    let parsed = parsing.snapshot(source, true, false)?;
    let tokens = parsed
        .tokens
        .as_deref()
        .expect("column specification tokens");
    let analysis = crate::source_analysis::SourceAnalysis::with_memo(
        source,
        &config.config,
        &parsed.protected,
        std::rc::Rc::clone(&parsed.metadata),
    );
    let environments =
        syntax::environment_ranges_with_tokens(source, tokens, &parsed.protected, &config.config)?;
    let mut declared = None;
    let mut declared_environments = None;
    let mut edits = Vec::new();
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for env in environments {
        let rule = config.config.alignment.rules.get(&env.name);
        let mode = rule
            .and_then(|rule| rule.column_spec)
            .unwrap_or(config.config.alignment.column_spec);
        if mode == ColumnSpecLayout::Preserve
            || crate::packages::column_spec_slot(&env.name).is_none()
            || config
                .config
                .syntax
                .environment_arguments
                .contains_key(&env.name)
            || syntax::is_protected(env.begin.start, &parsed.protected)
        {
            continue;
        }
        if declared_environments.is_none() {
            declared_environments = Some(analysis.declared_environments()?);
        }
        if declared_environments
            .as_ref()
            .is_some_and(|names| names.contains(&env.name))
        {
            continue;
        }
        let header_end = crate::layout::environment_header_end(
            source,
            env.body.clone(),
            &env.name,
            &config.config,
            &analysis,
        );
        let header = &source[env.body.start..header_end];
        let Some(Some(selected)) = header_columns(&env.name, header) else {
            continue;
        };
        let outer = selected.start + env.body.start - 1..selected.end + env.body.start + 1;
        let ray = crate::package_tables::is_tabularray(&env.name);
        let Some((local, grouped)) = preamble_range(header, selected, ray) else {
            continue;
        };
        let body = local.start + env.body.start..local.end + env.body.start;
        let protected = parsed
            .protected
            .partition_point(|range| range.end <= body.start);
        if parsed.protected[protected..]
            .iter()
            .take_while(|range| range.start < body.end)
            .any(|range| *range != outer)
            || has_format_directive(&source[outer])
        {
            continue;
        }
        let line = source[..body.start].rfind('\n').map_or(0, |at| at + 1);
        if mode == ColumnSpecLayout::Auto
            && !source[line..body.end]
                .lines()
                .any(|line| UnicodeWidthStr::width(line) > config.config.wrap.columns)
        {
            continue;
        }
        let text = &source[body.clone()];
        let declared = declared.get_or_insert_with(|| declared_types(source, tokens));
        let Some(columns) = layout_columns(text, declared, ray) else {
            continue;
        };
        if columns.len() < 2 {
            continue;
        }
        let indent: String = source[line..body.start]
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let unit = config.config.indent.unit("environment", &env.name);
        let mut replacement = String::new();
        let mut previous = 0;
        for (index, range) in columns.iter().enumerate() {
            let gap = &text[previous..range.start];
            if grouped || index > 0 {
                preamble_boundary(&mut replacement, gap, newline, &indent, &unit);
            } else {
                replacement.push_str(gap);
            }
            replacement.push_str(&text[range.clone()]);
            previous = range.end;
        }
        if grouped {
            preamble_boundary(&mut replacement, &text[previous..], newline, &indent, "");
        } else {
            replacement.push_str(&text[previous..]);
        }
        edits.push((body, replacement));
    }
    crate::layout::apply_edits(source, edits)
}

fn has_format_directive(source: &str) -> bool {
    // This pass alone may edit a table's built-in protected header. Disallow
    // directives even when their ranges are subsumed by that exact header.
    source.lines().any(|line| {
        line.split_once('%').is_some_and(|(_, comment)| {
            let Some((namespace, action)) = comment.trim().split_once(':') else {
                return false;
            };
            matches!(namespace, "tift" | "tex-fmt")
                && matches!(action.trim(), "off" | "on" | "skip")
        })
    })
}

fn preamble_boundary(output: &mut String, gap: &str, newline: &str, indent: &str, unit: &str) {
    if let Some(end) = gap.rfind('\n') {
        // Keep existing comments and paragraph boundaries. Only the indent of
        // the following column belongs to this layout stage.
        output.push_str(&gap[..end + 1]);
    } else {
        // Avoid inserting a TeX space token at a tight boundary.
        if gap.is_empty() {
            output.push('%');
        }
        output.push_str(newline);
    }
    output.push_str(indent);
    output.push_str(unit);
}

/// Parse the built-in header once for both row alignment and preamble layout.
/// `Some(None)` is a recognized header with an omitted optional argument.
fn header_columns(name: &str, header: &str) -> Option<Option<Range<usize>>> {
    if matches!(name, "tabu" | "longtabu") {
        let group = crate::package_tables::tabu_column_spec(header, 0..header.len())?;
        return Some(Some(group.start + 1..group.end - 1));
    }
    let signature = crate::packages::header_signature(name)?;
    let slot = crate::packages::column_spec_slot(name)?;
    let mut at = 0;
    let mut selected = None;
    for (index, kind) in signature.bytes().enumerate() {
        let before = at;
        at = skip(header, at);
        let body = match kind {
            b'o' | b'O'
                if header.as_bytes().get(at) == Some(&b'[') && (kind == b'o' || before == at) =>
            {
                group(header, at, b'[', b']')?
            }
            b'o' | b'O' => continue,
            b'm' => group(header, at, b'{', b'}')?,
            _ => return None,
        };
        at = body.end + 1;
        if index == slot {
            selected = Some(body);
        }
    }
    Some(selected)
}

fn preamble_range(header: &str, selected: Range<usize>, ray: bool) -> Option<(Range<usize>, bool)> {
    if !ray {
        return Some((selected, true));
    }
    // tabularray accepts a bare preamble, or inner options containing a
    // colspec value. The literal preamble parser distinguishes bare data from
    // arbitrary key lists. For a repeated key only the effective value matters.
    let mut result = None;
    let mut start = selected.start;
    for part in parts(&header[selected.clone()])? {
        let end = start + part.len();
        let key = skip(header, start);
        if key < end && header[key..end].starts_with("colspec") {
            let equal = skip(header, key + "colspec".len());
            if equal < end && header.as_bytes()[equal] == b'=' {
                let value = skip(header, equal + 1);
                let significant_end = start + part.trim_end().len();
                if value >= significant_end {
                    return None;
                }
                result = if header.as_bytes()[value] == b'{' {
                    let body = group(header, value, b'{', b'}')?;
                    if body.end >= end
                        || crate::trivia::skip_until(header, body.end + 1, end) != end
                    {
                        return None;
                    }
                    Some((body, true))
                } else {
                    Some((value..significant_end, false))
                };
            }
        }
        start = end + 1;
    }
    result.or(Some((selected, true)))
}

#[derive(Default)]
struct PreambleLayout {
    columns: Vec<Range<usize>>,
    pending: Option<Range<usize>>,
}

impl PreambleLayout {
    /// A prefix or intervening rule belongs to the next column. A trailing
    /// modifier belongs to the preceding one. Each repeat is one source unit.
    fn record(&mut self, kind: u8, span: Range<usize>) -> Option<()> {
        match kind {
            b'|' | b':' | b'>' | b'@' | b'!' => {
                self.pending.get_or_insert(span.clone()).end = span.end;
            }
            b'<' => {
                if self.pending.is_some() {
                    return None;
                }
                self.columns.last_mut()?.end = span.end;
            }
            _ => {
                let start = self
                    .pending
                    .take()
                    .map_or(span.start, |pending| pending.start);
                self.columns.push(start..span.end);
            }
        }
        Some(())
    }
}

fn layout_columns(source: &str, declared: &DeclaredTypes, ray: bool) -> Option<Vec<Range<usize>>> {
    let mut layout = PreambleLayout::default();
    if ray {
        tabularray::record_preamble(source, declared, &mut layout).ok()?;
    } else {
        parse_into(source, &mut Vec::new(), 0, declared, Some(&mut layout))?;
    }
    if let Some(pending) = layout.pending {
        layout.columns.last_mut()?.end = pending.end;
    }
    Some(layout.columns)
}

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
        let selector = matches!(name, "NewTblrChildSelector" | "NewTblrChildIndexer");
        let column = matches!(
            name,
            "newcolumntype"
                | "renewcolumntype"
                | "NewColumnType"
                | "NewTblrColumnType"
                | "NewTblrColumnRowType"
                | "NewColumnRowType"
        );
        let row = matches!(
            name,
            "NewRowType" | "NewTblrRowType" | "NewTblrColumnRowType" | "NewColumnRowType"
        );
        if (selector || column || row)
            && let Some(body) = group(source, skip(source, end), b'{', b'}')
        {
            let letter = source[body.clone()].trim();
            if selector {
                declared.selectors.insert(letter.to_owned());
            } else if let [byte] = letter.as_bytes() {
                // Complete local declarations take precedence over common
                // column defaults; opaque declarations keep the fallback.
                let literal = literal_column(source, body.end + 1);
                if column {
                    declared.columns.insert(*byte);
                    declared.literal_columns.remove(byte);
                    if let Some(column) = literal {
                        declared.literal_columns.insert(*byte, column);
                    }
                }
                if row {
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
    let selected = header_columns(name, header)?.map(|body| &header[body]);
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
    parse_into(source, &mut columns, 0, declared, None)?;
    (!columns.is_empty()).then_some(columns)
}

fn parse_into(
    source: &str,
    output: &mut Vec<Justification>,
    depth: usize,
    declared: &DeclaredTypes,
    mut layout: Option<&mut PreambleLayout>,
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
        let start = cursor;
        cursor += 1;
        if let Some(column) = declared.literal_columns.get(&byte) {
            for _ in 0..column.arguments {
                cursor = skip(source, cursor);
                cursor = group(source, cursor, b'{', b'}')?.end + 1;
            }
            if let Some(layout) = &mut layout {
                layout.record(byte, start..cursor)?;
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
            b'|' | b':' => {
                if let Some(layout) = &mut layout {
                    layout.record(byte, start..cursor)?;
                }
                continue;
            }

            b'>' | b'<' | b'@' | b'!' => {
                cursor = skip(source, cursor);
                cursor = group(source, cursor, b'{', b'}')?.end + 1;
                if let Some(layout) = &mut layout {
                    layout.record(byte, start..cursor)?;
                }
                continue;
            }
            b'*' => {
                cursor = skip(source, cursor);
                let count = group(source, cursor, b'{', b'}')?;
                let count_value = source[count.clone()].trim().parse::<usize>().ok()?;
                cursor = skip(source, count.end + 1);
                let body = group(source, cursor, b'{', b'}')?;
                let mut repeated = Vec::new();
                parse_into(
                    &source[body.clone()],
                    &mut repeated,
                    depth + 1,
                    declared,
                    None,
                )?;
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
                if let Some(layout) = &mut layout {
                    layout.record(byte, start..cursor)?;
                }
                continue;
            }
            // Custom column letters can consume arbitrary arguments. Their
            // arity cannot be inferred from the letter alone.
            _ => return None,
        };
        // The declared column type remains authoritative; modifier bodies
        // above are retained without evaluating arbitrary TeX commands.
        if let Some(layout) = &mut layout {
            layout.record(byte, start..cursor)?;
        }
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
