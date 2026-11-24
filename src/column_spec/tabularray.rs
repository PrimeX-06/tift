//! Literal tabularray inner keys, applied in their original order.

use super::{
    CellStyle, DeclaredTypes, Justification, MAX_COLUMNS, MAX_DEPTH, PreambleLayout, group, parts,
    skip,
};
use std::collections::HashSet;
use std::ops::Range;

const MAX_RULES: usize = 256;

pub(in crate::alignment) struct Spec {
    default: Justification,
    rules: Vec<Rule>,
    columns: usize,
}

enum Rule {
    Columns(Vec<CellStyle>),
    Rows(Vec<CellStyle>),
    Cells {
        selected: Vec<(Selector, Selector)>,
        style: CellStyle,
    },
}

/// Table commands are extracted in row-major source order after inner keys.
/// A later row or column command also applies to earlier source cells.
pub(crate) struct BodyStyles {
    rules: Vec<(BodySelection, CellStyle)>,
    invalid_rows: HashSet<usize>,
}

enum BodySelection {
    All,
    Row(usize),
    Column(usize),
    Cell(usize, usize),
}

impl BodyStyles {
    pub fn parse(
        source: &str,
        opaque: &[Range<usize>],
        cells: impl Iterator<Item = (usize, usize, Range<usize>)>,
    ) -> Result<Self, ()> {
        let mut result = Self {
            rules: Vec::new(),
            invalid_rows: HashSet::new(),
        };
        for (row, column, range) in cells {
            let text = &source[range.clone()];
            let mut cursor = 0;
            while cursor < text.len() {
                cursor = skip(text, cursor);
                let absolute = range.start + cursor;
                let protected = opaque.partition_point(|item| item.end <= absolute);
                if let Some(item) = opaque.get(protected)
                    && item.start <= absolute
                    && absolute < item.end
                {
                    cursor = item.end.min(range.end) - range.start;
                    continue;
                }
                match text.as_bytes().get(cursor) {
                    Some(b'{') => {
                        cursor = group(text, cursor, b'{', b'}').ok_or(())?.end + 1;
                    }
                    Some(b'\\') => {
                        let (name, end) = crate::syntax::command_at(text, cursor).ok_or(())?;
                        cursor = end;
                        if name == "begin" {
                            cursor = nested_environment_end(text, cursor)?;
                            continue;
                        }
                        if !super::TABULARRAY_STYLE_COMMANDS.contains(&name) {
                            continue;
                        }
                        let parsed = body_command(text, cursor, name);
                        let (style, end) = match parsed {
                            Ok(value) => value,
                            Err(()) if name == "SetCell" => {
                                result.invalid_rows.insert(row);
                                break;
                            }
                            Err(()) => return Err(()),
                        };
                        cursor = end;
                        let selection = match name {
                            "SetRow" => BodySelection::Row(row),
                            "SetColumn" => BodySelection::Column(column),
                            "SetCell" => BodySelection::Cell(row, column),
                            _ => BodySelection::All,
                        };
                        result.rules.push((selection, style));
                        if result.rules.len() > MAX_COLUMNS {
                            return Err(());
                        }
                    }
                    Some(_) => cursor += text[cursor..].chars().next().ok_or(())?.len_utf8(),
                    None => break,
                }
            }
        }
        Ok(result)
    }

    pub fn row_supported(&self, row: usize) -> bool {
        !self.invalid_rows.contains(&row)
    }

    pub fn style(&self, row: usize, column: usize) -> CellStyle {
        let mut result = CellStyle::default();
        for (selection, style) in &self.rules {
            let selected = match *selection {
                BodySelection::All => true,
                BodySelection::Row(selected) => selected == row,
                BodySelection::Column(selected) => selected == column,
                BodySelection::Cell(selected_row, selected_column) => {
                    selected_row == row && selected_column == column
                }
            };
            if selected {
                result.overlay(*style);
            }
        }
        result
    }
}

fn nested_environment_end(source: &str, start: usize) -> Result<usize, ()> {
    let name = group(source, skip(source, start), b'{', b'}').ok_or(())?;
    let expected = &source[name.clone()];
    let mut cursor = name.end + 1;
    let mut depth = 1usize;
    while cursor < source.len() {
        cursor = skip(source, cursor);
        match source.as_bytes().get(cursor) {
            Some(b'{') => cursor = group(source, cursor, b'{', b'}').ok_or(())?.end + 1,
            Some(b'\\') => {
                let (name, end) = crate::syntax::command_at(source, cursor).ok_or(())?;
                cursor = end;
                if matches!(name, "begin" | "end") {
                    let environment = group(source, skip(source, cursor), b'{', b'}').ok_or(())?;
                    if &source[environment.clone()] == expected {
                        if name == "begin" {
                            depth += 1;
                            if depth > MAX_DEPTH {
                                return Err(());
                            }
                        } else {
                            depth -= 1;
                            if depth == 0 {
                                return Ok(environment.end + 1);
                            }
                        }
                    }
                    cursor = environment.end + 1;
                }
            }
            Some(_) => cursor += source[cursor..].chars().next().ok_or(())?.len_utf8(),
            None => break,
        }
    }
    Err(())
}

fn body_command(source: &str, start: usize, name: &str) -> Result<(CellStyle, usize), ()> {
    let cell = matches!(name, "SetCell" | "SetCells");
    let mut style = CellStyle {
        colspan: cell.then_some(1),
        rowspan: cell.then_some(1),
        ..CellStyle::default()
    };
    let mut cursor = skip(source, start);
    if source.as_bytes().get(cursor) == Some(&b'[') {
        let options = group(source, cursor, b'[', b']').ok_or(())?;
        if cell {
            style.overlay(span_keys(&source[options.clone()])?);
        }
        cursor = skip(source, options.end + 1);
    }
    let body = group(source, cursor, b'{', b'}').ok_or(())?;
    style.overlay(alignment_keys(&source[body.clone()])?);
    Ok((style, body.end + 1))
}

impl Spec {
    pub fn parse(
        source: &str,
        default: Justification,
        declared: &DeclaredTypes,
    ) -> Result<Self, ()> {
        let mut result = Self {
            default,
            rules: Vec::new(),
            columns: 0,
        };
        let source = source.trim();
        if source.is_empty() {
            return Ok(result);
        }
        if let Ok(columns) = preamble(source, declared) {
            result.columns = columns.len();
            result.rules.push(Rule::Columns(columns));
            return Ok(result);
        }
        for entry in parts(source).ok_or(())? {
            let entry = clean(entry);
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (key, value) = entry.split_once('=').ok_or(())?;
            let key = key.trim();
            let value = value.trim();
            match key {
                "colspec" => {
                    let columns = preamble(unbrace(value)?, declared)?;
                    result.columns = columns.len();
                    result.rules.push(Rule::Columns(columns));
                }
                "rowspec" => result
                    .rules
                    .push(Rule::Rows(preamble(unbrace(value)?, &declared.for_rows())?)),
                "columns" | "rows" | "cells" => {
                    let style = style_value(value, key == "cells")?;
                    result.rules.push(Rule::Cells {
                        selected: vec![(Selector::all(), Selector::all())],
                        style,
                    });
                }
                _ if key.starts_with("column{")
                    || key.starts_with("row{")
                    || key.starts_with("cell{") =>
                {
                    result.rules.push(Rule::Cells {
                        selected: key_selection(key, declared)?,
                        style: style_value(value, key.starts_with("cell{"))?,
                    });
                }
                // These affect dimensions, rules, content metadata or outer
                // table placement. Their grouped values stay opaque here.
                "name" | "width" | "hspan" | "vspan" | "stretch" | "hlines" | "vlines"
                | "leftsep" | "rightsep" | "colsep" | "abovesep" | "belowsep" | "rowsep"
                | "rulesep" | "rowhead" | "rowfoot" | "delimiter" | "baseline" => {}
                _ if ["hline{", "vline{", "hborder{", "vborder{"]
                    .iter()
                    .any(|prefix| key.starts_with(prefix)) => {}
                _ => return Err(()),
            }
            if result.rules.len() > MAX_RULES {
                return Err(());
            }
        }
        Ok(result)
    }

    pub fn column_count(&self) -> usize {
        self.columns
    }

    pub fn style(&self, row: usize, column: usize, rows: usize, columns: usize) -> CellStyle {
        let mut result = CellStyle {
            justification: Some(self.default),
            ..CellStyle::default()
        };
        for rule in &self.rules {
            match rule {
                Rule::Columns(values) => {
                    if let Some(style) = values.get(column - 1) {
                        result.overlay(*style);
                    }
                }
                Rule::Rows(values) => {
                    if let Some(style) = values.get(row - 1) {
                        result.overlay(*style);
                    }
                }
                Rule::Cells { selected, style } => {
                    if selected.iter().any(|(selected_rows, selected_columns)| {
                        selected_rows.matches(row, rows)
                            && selected_columns.matches(column, columns)
                    }) {
                        result.overlay(*style);
                    }
                }
            }
        }
        result
    }
}

fn clean(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'%' => {
                cursor = super::comment_end(source, cursor);
                output.push(' ');
            }
            b'\\' => {
                let end =
                    crate::syntax::command_at(source, cursor).map_or(cursor + 1, |(_, end)| end);
                output.push_str(&source[cursor..end]);
                cursor = end;
            }
            _ => {
                let character = source[cursor..].chars().next().expect("source character");
                output.push(character);
                cursor += character.len_utf8();
            }
        }
    }
    output
}

fn unbrace(source: &str) -> Result<&str, ()> {
    let source = source.trim();
    if !source.starts_with('{') {
        return Ok(source);
    }
    let body = group(source, 0, b'{', b'}').ok_or(())?;
    if skip(source, body.end + 1) != source.len() {
        return Err(());
    }
    Ok(&source[body])
}

fn style_value(source: &str, cell: bool) -> Result<CellStyle, ()> {
    let source = unbrace(source)?.trim();
    let mut result = CellStyle {
        colspan: cell.then_some(1),
        rowspan: cell.then_some(1),
        ..CellStyle::default()
    };
    if source.starts_with('{') {
        let first = group(source, 0, b'{', b'}').ok_or(())?;
        let cursor = skip(source, first.end + 1);
        if cursor == source.len() {
            result.overlay(alignment_keys(&source[first])?);
            return Ok(result);
        }
        let second = group(source, cursor, b'{', b'}').ok_or(())?;
        if skip(source, second.end + 1) != source.len() {
            return Err(());
        }
        // Row/column commands currently ignore their first argument in the
        // fixed package grammar. Only a cell's outer argument defines spans.
        if cell {
            result.overlay(span_keys(&source[first])?);
        }
        result.overlay(alignment_keys(&source[second])?);
        return Ok(result);
    }
    result.overlay(alignment_keys(source)?);
    Ok(result)
}

fn alignment_keys(source: &str) -> Result<CellStyle, ()> {
    let mut style = CellStyle::default();
    for option in parts(source).ok_or(())? {
        let option = option.trim();
        if option.is_empty() {
            continue;
        }
        let (key, value) = option
            .split_once('=')
            .map_or((option, None), |(key, value)| {
                (key.trim(), Some(value.trim()))
            });
        match (key, value) {
            ("halign", Some(value)) => style.justification = Some(alignment(unbrace(value)?)?),
            ("l" | "c" | "r" | "j", None) => style.justification = Some(alignment(key)?),
            ("l" | "c" | "r" | "j" | "halign", _) => return Err(()),
            (
                "valign" | "wd" | "ht" | "co" | "bg" | "fg" | "font" | "mode" | "cmd" | "preto"
                | "appto" | "si" | "leftsep" | "rightsep" | "abovesep" | "belowsep" | "rowsep"
                | "colsep" | "colsep+" | "rowsep+" | "baseline" | "leftsep+" | "rightsep+"
                | "abovesep+" | "belowsep+" | "leftspace" | "rightspace" | "abovespace"
                | "belowspace" | "leftspace+" | "rightspace+" | "abovespace+" | "belowspace+",
                _,
            ) => {}
            ("t" | "p" | "m" | "b" | "h" | "f" | "$" | "$$", None) => {}
            // Unkeyed literal dimensions, coefficients and colors are allowed
            // by tabularray's own unknown-key handler. TeX commands require
            // expansion and cannot be classified as harmless style metadata.
            (_, None)
                if key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"!.-+ ".contains(&byte)) => {}
            _ => return Err(()),
        }
    }
    Ok(style)
}

fn alignment(source: &str) -> Result<Justification, ()> {
    match source.trim() {
        "l" | "j" => Ok(Justification::Left),
        "c" => Ok(Justification::Center),
        "r" => Ok(Justification::Right),
        _ => Err(()),
    }
}

fn span_keys(source: &str) -> Result<CellStyle, ()> {
    let mut result = CellStyle::default();
    for option in parts(source).ok_or(())? {
        let option = option.trim();
        if option.is_empty() {
            continue;
        }
        let (key, value) = option.split_once('=').ok_or(())?;
        let count = unbrace(value)?.trim().parse::<usize>().map_err(|_| ())?;
        if !(1..=MAX_COLUMNS).contains(&count) {
            return Err(());
        }
        match key.trim() {
            "c" => result.colspan = Some(count),
            "r" => result.rowspan = Some(count),
            _ => return Err(()),
        }
    }
    Ok(result)
}

fn preamble(source: &str, declared: &DeclaredTypes) -> Result<Vec<CellStyle>, ()> {
    let mut result = Vec::new();
    preamble_into(source, declared, &mut result, 0, None)?;
    if result.is_empty() {
        return Err(());
    }
    Ok(result)
}

pub(super) fn record_preamble(
    source: &str,
    declared: &DeclaredTypes,
    layout: &mut PreambleLayout,
) -> Result<(), ()> {
    preamble_into(source, declared, &mut Vec::new(), 0, Some(layout))
}

fn preamble_into(
    source: &str,
    declared: &DeclaredTypes,
    result: &mut Vec<CellStyle>,
    depth: usize,
    mut layout: Option<&mut PreambleLayout>,
) -> Result<(), ()> {
    if depth >= MAX_DEPTH {
        return Err(());
    }
    let mut cursor = 0;
    while cursor < source.len() {
        cursor = skip(source, cursor);
        let Some(&kind) = source.as_bytes().get(cursor) else {
            break;
        };
        let start = cursor;
        cursor += 1;
        // When parsing optional arguments, do not include trailing trivia in
        // the lexical extent of a column or rule that has no optional group.
        let mut end = cursor;
        if let Some(column) = declared.literal_columns.get(&kind) {
            for _ in 0..column.arguments {
                cursor = skip(source, cursor);
                cursor = group(source, cursor, b'{', b'}').ok_or(())?.end + 1;
            }
            if let Some(layout) = &mut layout {
                layout.record(kind, start..cursor).ok_or(())?;
            }
            result.push(column.style);
            if result.len() > MAX_COLUMNS {
                return Err(());
            }
            continue;
        }
        if declared.columns.contains(&kind) {
            return Err(());
        }
        let style = match kind {
            b'l' | b'c' | b'r' | b'j' => CellStyle {
                justification: Some(alignment(&source[cursor - 1..cursor])?),
                ..CellStyle::default()
            },
            b'L' | b'C' | b'R' => {
                let justification = match kind {
                    b'C' => Justification::Center,
                    b'R' => Justification::Right,
                    _ => Justification::Left,
                };
                let mut style = CellStyle {
                    justification: Some(justification),
                    ..CellStyle::default()
                };
                cursor = skip(source, cursor);
                if source.as_bytes().get(cursor) == Some(&b'[') {
                    let options = group(source, cursor, b'[', b']').ok_or(())?;
                    style.overlay(alignment_keys(&source[options.clone()])?);
                    end = options.end + 1;
                    cursor = skip(source, end);
                }
                if source.as_bytes().get(cursor) == Some(&b'{') {
                    cursor = group(source, cursor, b'{', b'}').ok_or(())?.end + 1;
                    end = cursor;
                }
                style
            }
            b'Q' | b'X' | b'S' | b's' => {
                cursor = skip(source, cursor);
                let mut style = if matches!(kind, b'S' | b's') {
                    CellStyle {
                        justification: Some(Justification::Center),
                        decimal: (kind == b'S').then_some(&['.', ','][..]),
                        ..CellStyle::default()
                    }
                } else {
                    CellStyle::default()
                };
                if source.as_bytes().get(cursor) == Some(&b'[') {
                    let options = group(source, cursor, b'[', b']').ok_or(())?;
                    if !matches!(kind, b'S' | b's') {
                        style.overlay(alignment_keys(&source[options.clone()])?);
                    }
                    cursor = options.end + 1;
                    end = cursor;
                }
                style
            }
            b't' | b'p' | b'm' | b'b' | b'h' | b'f' => {
                cursor = skip(source, cursor);
                cursor = group(source, cursor, b'{', b'}').ok_or(())?.end + 1;
                end = cursor;
                CellStyle::default()
            }
            b'|' | b':' => {
                cursor = skip(source, cursor);
                if source.as_bytes().get(cursor) == Some(&b'[') {
                    cursor = group(source, cursor, b'[', b']').ok_or(())?.end + 1;
                    end = cursor;
                }
                if let Some(layout) = &mut layout {
                    layout.record(kind, start..end).ok_or(())?;
                }
                continue;
            }
            b'>' | b'<' | b'@' | b'!' => {
                cursor = skip(source, cursor);
                if source.as_bytes().get(cursor) == Some(&b'[') {
                    cursor = skip(source, group(source, cursor, b'[', b']').ok_or(())?.end + 1);
                }
                cursor = group(source, cursor, b'{', b'}').ok_or(())?.end + 1;
                if let Some(layout) = &mut layout {
                    layout.record(kind, start..cursor).ok_or(())?;
                }
                continue;
            }
            b'*' => {
                cursor = skip(source, cursor);
                let count = group(source, cursor, b'{', b'}').ok_or(())?;
                let count_value = source[count.clone()]
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| ())?;
                cursor = skip(source, count.end + 1);
                let body = group(source, cursor, b'{', b'}').ok_or(())?;
                let mut repeated = Vec::new();
                preamble_into(
                    &source[body.clone()],
                    declared,
                    &mut repeated,
                    depth + 1,
                    None,
                )?;
                let additional = repeated.len().checked_mul(count_value).ok_or(())?;
                if result.len().checked_add(additional).ok_or(())? > MAX_COLUMNS {
                    return Err(());
                }
                if !repeated.is_empty() {
                    for _ in 0..count_value {
                        result.extend_from_slice(&repeated);
                    }
                }
                cursor = body.end + 1;
                if let Some(layout) = &mut layout {
                    layout.record(kind, start..cursor).ok_or(())?;
                }
                continue;
            }
            _ => return Err(()),
        };
        if let Some(layout) = &mut layout {
            layout.record(kind, start..end).ok_or(())?;
        }
        result.push(style);
        if result.len() > MAX_COLUMNS {
            return Err(());
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Selector(Vec<Part>);

#[derive(Clone)]
enum Part {
    All,
    Range(Index, Index),
    One(Index),
    Parity {
        odd: bool,
        first: usize,
    },
    Every {
        step: usize,
        first: Index,
        last: Index,
    },
}

#[derive(Clone, Copy)]
enum Index {
    Literal(usize),
    Last(usize),
}

impl Index {
    fn resolve(self, total: usize) -> usize {
        match self {
            Self::Literal(value) => value,
            Self::Last(offset) => total.saturating_add(1).saturating_sub(offset).max(1),
        }
    }
}

impl Selector {
    fn all() -> Self {
        Self(vec![Part::All])
    }

    fn parse(source: &str, declared: &DeclaredTypes) -> Result<Self, ()> {
        let mut result = Vec::new();
        for item in parts(source).ok_or(())? {
            let item = item.trim();
            if item == "-" {
                result.push(Part::All);
            } else if item.starts_with("odd") || item.starts_with("even") {
                let odd = item.starts_with("odd");
                let name = if odd { "odd" } else { "even" };
                if declared.selectors.contains(name) {
                    return Err(());
                }
                let mut cursor = name.len();
                let first = optional_integer(item, &mut cursor)?.unwrap_or(if odd { 1 } else { 2 });
                if skip(item, cursor) != item.len() || first == 0 {
                    return Err(());
                }
                result.push(Part::Parity { odd, first });
            } else if item.starts_with("every") {
                if declared.selectors.contains("every") {
                    return Err(());
                }
                let mut cursor = 5;
                let step = optional_integer(item, &mut cursor)?.unwrap_or(1);
                if !(1..=MAX_COLUMNS).contains(&step) {
                    return Err(());
                }
                cursor = skip(item, cursor);
                let first = group(item, cursor, b'{', b'}').ok_or(())?;
                cursor = skip(item, first.end + 1);
                let last = group(item, cursor, b'{', b'}').ok_or(())?;
                if skip(item, last.end + 1) != item.len() {
                    return Err(());
                }
                result.push(Part::Every {
                    step,
                    first: index(&item[first], declared)?,
                    last: index(&item[last], declared)?,
                });
            } else if let Some((first, last)) = item
                .split_once('-')
                .filter(|(first, _)| !first.trim().is_empty())
            {
                result.push(Part::Range(index(first, declared)?, index(last, declared)?));
            } else {
                result.push(Part::One(index(item, declared)?));
            }
            if result.len() > MAX_COLUMNS {
                return Err(());
            }
        }
        if result.is_empty() {
            return Err(());
        }
        Ok(Self(result))
    }

    fn matches(&self, position: usize, total: usize) -> bool {
        self.0.iter().any(|part| match *part {
            Part::All => true,
            Part::One(index) => position == index.resolve(total),
            Part::Range(first, last) => {
                (first.resolve(total)..=last.resolve(total)).contains(&position)
            }
            Part::Parity { odd, first } => position >= first && (position % 2 == 1) == odd,
            Part::Every { step, first, last } => {
                let first = first.resolve(total);
                position >= first
                    && position <= last.resolve(total)
                    && (position - first).is_multiple_of(step)
            }
        })
    }
}

fn optional_integer(source: &str, cursor: &mut usize) -> Result<Option<usize>, ()> {
    *cursor = skip(source, *cursor);
    if source.as_bytes().get(*cursor) != Some(&b'[') {
        return Ok(None);
    }
    let body = group(source, *cursor, b'[', b']').ok_or(())?;
    *cursor = body.end + 1;
    Ok(Some(source[body].trim().parse::<usize>().map_err(|_| ())?))
}

fn index(source: &str, declared: &DeclaredTypes) -> Result<Index, ()> {
    let source = source.trim();
    if let Ok(value) = source.parse::<isize>() {
        if value == 0 || value.unsigned_abs() > MAX_COLUMNS {
            return Err(());
        }
        return Ok(if value < 0 {
            Index::Last(value.unsigned_abs())
        } else {
            Index::Literal(value as usize)
        });
    }
    let Some(&letter) = source.as_bytes().first() else {
        return Err(());
    };
    if !(b'U'..=b'Z').contains(&letter) || declared.selectors.contains(&source[..1]) {
        return Err(());
    }
    let mut cursor = 1;
    let optional = optional_integer(source, &mut cursor)?;
    if skip(source, cursor) != source.len() || optional.is_some() && letter != b'Z' {
        return Err(());
    }
    let offset = optional.unwrap_or((b'Z' - letter + 1) as usize);
    if !(1..=MAX_COLUMNS).contains(&offset) {
        return Err(());
    }
    Ok(Index::Last(offset))
}

fn key_selection(source: &str, declared: &DeclaredTypes) -> Result<Vec<(Selector, Selector)>, ()> {
    let (name, mut cursor) = source
        .find('{')
        .map(|position| (&source[..position], position))
        .ok_or(())?;
    let first = group(source, cursor, b'{', b'}').ok_or(())?;
    cursor = skip(source, first.end + 1);
    match name {
        "row" | "column" if cursor == source.len() => {
            let selected = Selector::parse(&source[first], declared)?;
            Ok(if name == "row" {
                vec![(selected, Selector::all())]
            } else {
                vec![(Selector::all(), selected)]
            })
        }
        "cell" if cursor < source.len() => {
            let second = group(source, cursor, b'{', b'}').ok_or(())?;
            if skip(source, second.end + 1) != source.len() {
                return Err(());
            }
            Ok(vec![(
                Selector::parse(&source[first], declared)?,
                Selector::parse(&source[second], declared)?,
            )])
        }
        "cell" => {
            let mut result = Vec::new();
            for pair in parts(&source[first]).ok_or(())? {
                let pair = pair.trim();
                let row = group(pair, 0, b'{', b'}').ok_or(())?;
                let column = group(pair, skip(pair, row.end + 1), b'{', b'}').ok_or(())?;
                if skip(pair, column.end + 1) != pair.len() {
                    return Err(());
                }
                result.push((
                    Selector::parse(&pair[row], declared)?,
                    Selector::parse(&pair[column], declared)?,
                ));
                if result.len() > MAX_RULES {
                    return Err(());
                }
            }
            if result.is_empty() {
                return Err(());
            }
            Ok(result)
        }
        _ => Err(()),
    }
}
