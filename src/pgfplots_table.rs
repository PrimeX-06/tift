//! Source columns for the whitespace-separated records of a literal plot table.

use crate::{CompiledConfig, FormatError, parse_context::ParseContext, syntax};
use unicode_width::UnicodeWidthStr;

fn slash_rows(options: &str) -> Option<bool> {
    let mut slash = false;
    for option in crate::alignment::comma_parts(options)? {
        let (key, value) = option.trim().split_once('=').unwrap_or((option.trim(), ""));
        let value = value.trim();
        let value = value
            .strip_prefix('{')
            .and_then(|v| v.strip_suffix('}'))
            .unwrap_or(value);
        let key = key.trim().trim_start_matches("/pgfplots/table/");
        match key {
            "row sep" => {
                slash = match value {
                    "newline" => false,
                    r"\\" | "crcr" => true,
                    _ => return None,
                }
            }
            "col sep" if value == "space" => (),
            "format" if value == "inline" || value == "auto" => (),
            "" | "x" | "y" | "z" | "meta" | "x index" | "y index" | "z index" | "meta index"
            | "x expr" | "y expr" | "z expr" | "meta expr" | "header" => (),
            // Other table readers and custom styles may assign meaning to spaces.
            _ => return None,
        }
    }
    Some(slash)
}

fn cells(row: &str) -> Option<Vec<&str>> {
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < row.len() {
        let start = pos;
        match row.as_bytes()[pos] {
            b'{' => pos = syntax::balanced_end(row, pos, b'{', b'}')?,
            b' ' | b'\r' | b'\n' | b'\t' => {
                pos += 1;
                continue;
            }
            _ => {
                let len = row[pos..]
                    .find(|c: char| c.is_ascii_whitespace())
                    .unwrap_or(row.len() - pos);
                if len == 0
                    || row[pos..pos + len].contains(['{', '}', '\\', ',', ';', ':', '&', '~', '^'])
                {
                    return None;
                }
                pos += len;
            }
        }
        if row
            .as_bytes()
            .get(pos)
            .is_some_and(|c| !c.is_ascii_whitespace())
        {
            return None;
        }
        result.push(&row[start..pos]);
    }
    Some(result)
}

fn measure_fields(widths: &mut Vec<usize>, fields: &[&str]) -> Option<()> {
    if widths.is_empty() {
        widths.resize(fields.len(), 0);
    }
    if widths.len() != fields.len() {
        return None;
    }
    for (width, field) in widths.iter_mut().zip(fields) {
        *width = (*width).max(UnicodeWidthStr::width(*field));
    }
    Some(())
}

fn write_fields(output: &mut String, fields: &[&str], widths: &[usize]) {
    for (column, field) in fields.iter().enumerate() {
        output.push_str(field);
        if column + 1 < fields.len() {
            output.extend(std::iter::repeat_n(
                ' ',
                widths[column] - UnicodeWidthStr::width(*field) + 1,
            ));
        }
    }
}

fn render(data: &str, slash: bool, base: &str, unit: &str, newline: &str) -> Option<String> {
    if data.contains(['%', '#'])
        || data
            .lines()
            .any(|line| line.trim_start_matches([' ', '\t']).contains('\t'))
    {
        return None;
    }
    let data = data.trim_matches([' ', '\t', '\r', '\n']);
    let sep = if slash { r"\\" } else { "\n" };
    let final_separator = slash && data.ends_with(sep);
    let mut rows = Vec::new();
    let mut widths = Vec::new();
    for row in data.split_terminator(sep) {
        let fields = cells(row)?;
        if !fields.is_empty() {
            measure_fields(&mut widths, &fields)?;
        }
        rows.push(fields);
    }
    if widths.len() < 2 || rows.len() < 2 {
        return None;
    }
    let mut output = String::new();
    for (index, row) in rows.iter().enumerate() {
        output.push_str(newline);
        if row.is_empty() && !slash {
            continue;
        }
        output.push_str(base);
        output.push_str(unit);
        write_fields(&mut output, row, &widths);
        if slash && (index + 1 < rows.len() || final_separator) {
            output.push_str(r"\\");
        }
    }
    output.push_str(newline);
    output.push_str(base);
    Some(output)
}

// The first and last records can share their lines with the surrounding
// braces. Preserve every physical boundary of a named table.
fn named_rows(data: &str, slash: bool, base: &str, unit: &str) -> Option<String> {
    if !data.contains('\n') || data.contains(['%', '#']) {
        return None;
    }
    let mut widths = Vec::<usize>::new();
    let mut rows = Vec::new();
    let mut previous_terminated = true;
    let mut records = 0;
    for line in data.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let value = body.trim_matches([' ', '\t']);
        if value.is_empty() {
            rows.push((line, body, Vec::new(), false));
            continue;
        }
        if !previous_terminated || value.contains('\t') {
            return None;
        }
        let stripped = slash.then(|| value.strip_suffix(r"\\")).flatten();
        let value = stripped.unwrap_or(value);
        if slash && value.contains(r"\\") {
            return None;
        }
        let fields = cells(value)?;
        if fields.len() < 2 {
            return None;
        }
        measure_fields(&mut widths, &fields)?;
        records += 1;
        previous_terminated = !slash || stripped.is_some();
        rows.push((line, body, fields, stripped.is_some()));
    }
    if records < 2 {
        return None;
    }
    let mut output = String::with_capacity(data.len());
    for (index, (line, body, fields, terminated)) in rows.into_iter().enumerate() {
        if fields.is_empty() {
            if index > 0 && !line.ends_with('\n') {
                output.push_str(base);
            }
        } else {
            if index > 0 {
                output.push_str(base);
                output.push_str(unit);
            } else {
                output.push_str(&body[..body.len() - body.trim_start_matches([' ', '\t']).len()]);
            }
            write_fields(&mut output, &fields, &widths);
            if terminated {
                output.push_str(r"\\");
            }
        }
        output.push_str(&line[body.len()..]);
    }
    if data.ends_with('\n') {
        output.push_str(base);
    }
    Some(output)
}

pub(crate) fn table_edits(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<crate::edit_plan::RuleEdits, FormatError> {
    let parsed = parsing.snapshot(source, true, false)?;
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let unit = config.config.indent.unit("tikz-path", "addplot");
    let mut plan = crate::edit_plan::RuleEdits::default();
    let mut inherited_reader = false;
    for token in parsed.tokens.as_deref().expect("plot tokens") {
        if token.kind != syntax::TokenKind::Command {
            continue;
        }
        let name = &source[token.span.clone()];
        if matches!(name, r"\pgfplotstableset" | r"\pgfplotsset" | r"\pgfkeys") {
            let arg = crate::trivia::skip(source, token.span.end);
            if let Some(end) = syntax::balanced_end(source, arg, b'{', b'}') {
                plan.reads.push(token.span.start..end);
                let options = &source[arg + 1..end - 1];
                inherited_reader |= match name {
                    r"\pgfplotstableset" => {
                        crate::alignment::comma_parts(options).is_none_or(|parts| {
                            parts.iter().any(|part| {
                                let key = part.trim().split('=').next().unwrap_or("").trim();
                                !matches!(
                                    key,
                                    "" | "columns"
                                        | "precision"
                                        | "fixed"
                                        | "sci"
                                        | "fixed zerofill"
                                ) && !key.starts_with("columns/")
                                    && !key.starts_with("every head row/")
                                    && !key.starts_with("every last row/")
                            })
                        })
                    }
                    _ => options.contains("table/") || options.contains("/pgfplots/table"),
                };
            }
            continue;
        }
        let named = name == r"\pgfplotstableread";
        let args = match name {
            r"\addplot" if config.config.breaks.pgfplots_tables => {
                crate::packages::circuits_timing::plot_table(source, token.span.end)
            }
            r"\pgfplotstableread"
                if config.config.breaks.pgfplots_named_tables && !inherited_reader =>
            {
                crate::packages::circuits_timing::table_arguments(source, token.span.end)
            }
            _ => continue,
        };
        let Some((options, data)) = args else {
            continue;
        };
        let line_start = source[..token.span.start]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        let read_end = if named {
            source[data.end..]
                .find('\n')
                .map_or(source.len(), |at| data.end + at)
        } else {
            data.end
        };
        plan.reads.push(line_start..read_end);
        // The lexical pass assigns this range only to a recognized package reader.
        let owner = options.start..data.end;
        let index = parsed.protected.partition_point(|r| r.end <= owner.start);
        if parsed.protected.get(index) != Some(&owner) {
            continue;
        }
        let options = &source[options];
        let Some(slash) = slash_rows(
            options
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or(""),
        ) else {
            continue;
        };
        let line = source[..token.span.start].rsplit('\n').next().unwrap_or("");
        let base = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        let body = data.start + 1..data.end - 1;
        if named && (line != base || !standalone_target(&source[data.end..])) {
            continue;
        }
        let rendered = if named {
            named_rows(&source[body.clone()], slash, base, &unit)
        } else {
            render(&source[body.clone()], slash, base, &unit, newline)
        };
        if let Some(text) = rendered
            && text != source[body.clone()]
        {
            plan.edits.replace(body, text);
        }
    }
    Ok(plan)
}

fn standalone_target(rest: &str) -> bool {
    let target = rest.trim_start_matches([' ', '\t', '\r', '\n']);
    let Some((_, end)) = syntax::command_at(target, 0) else {
        return false;
    };
    let tail = target[end..].split('\n').next().unwrap_or("").trim();
    tail.is_empty() || tail.starts_with('%')
}
