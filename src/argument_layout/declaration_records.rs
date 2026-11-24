//! Layout of literal records within standalone declarations.

use std::ops::Range;

pub(super) fn glossary_data(
    groups: &[Range<usize>],
    protected: &[Range<usize>],
    command: &Range<usize>,
) -> bool {
    if groups.len() != 2 {
        return false;
    }
    let first = protected.partition_point(|span| span.end <= command.start);
    protected[first..]
        .iter()
        .take_while(|span| span.start < command.end)
        .all(|span| {
            groups.contains(span)
                || (span.start == groups[0].start
                    && span.end == groups[1].end
                    && groups[0].end == groups[1].start)
        })
}

pub(super) fn format(
    name: &str,
    body: &str,
    base: &str,
    unit: &str,
    newline: &str,
    expand: bool,
    columns: usize,
) -> Option<String> {
    let rows = match name {
        "newglossaryentry" if expand => glossary(body)?,
        "hyphenation" => {
            let words: Vec<_> = body.split_ascii_whitespace().collect();
            if (!expand && words.len() < 4)
                || !words
                    .iter()
                    .all(|word| word.chars().all(|c| c.is_alphabetic() || c == '-'))
            {
                return None;
            }
            words.into_iter().map(str::to_owned).collect()
        }
        "DeclareFontShape" => font_sizes(body, columns)?,
        _ => return None,
    };
    if rows.len() < 2 {
        return None;
    }
    let mut out = String::new();
    for row in rows {
        out.push_str(newline);
        out.push_str(base);
        out.push_str(unit);
        out.push_str(&row);
    }
    out.push_str(newline);
    out.push_str(base);
    Some(out)
}

fn glossary(body: &str) -> Option<Vec<String>> {
    // A comment may suppress a token separator or control a formatter region.
    if body
        .match_indices('%')
        .any(|(at, _)| !crate::syntax::is_escaped(body, at))
    {
        return None;
    }
    let mut rows = Vec::new();
    for span in crate::packages::keys::definition_entries(body) {
        let raw = &body[span];
        let end = raw
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .len();
        if end != raw.len() && crate::syntax::is_escaped(raw, end) {
            return None;
        }
        let entry = raw[..end].trim_start_matches(|c: char| c.is_ascii_whitespace());
        if entry.is_empty() {
            continue;
        }
        let (key, value) = entry.trim_end_matches(',').split_once('=')?;
        if key.trim().is_empty()
            || !key
                .trim()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.trim().is_empty()
        {
            return None;
        }
        rows.push(entry.to_owned());
    }
    Some(rows)
}

fn valid_size(selector: &str) -> bool {
    let number = |s: &str| {
        !s.is_empty()
            && s.bytes().any(|b| b.is_ascii_digit())
            && s.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            && s.matches('.').count() <= 1
    };
    if let Some((from, to)) = selector.split_once('-') {
        (from.is_empty() || number(from)) && (to.is_empty() || number(to))
    } else {
        number(selector)
    }
}

fn font_sizes(body: &str, columns: usize) -> Option<Vec<String>> {
    // The NFSS table may use % to suppress a line ending between selectors.
    // Keep authored comments and executable font specifications opaque.
    if body.split_inclusive('\n').any(|line| {
        line.split_once('%')
            .is_some_and(|(_, comment)| !comment.trim().is_empty())
    }) {
        return None;
    }
    let mut pos = crate::trivia::skip(body, 0);
    let mut records = Vec::new();
    let mut widest = 0;
    while pos < body.len() {
        let mut sizes = String::new();
        while body[pos..].starts_with('<') {
            let end = pos + body[pos..].find('>')?;
            if !valid_size(&body[pos + 1..end]) {
                return None;
            }
            sizes.push_str(&body[pos..=end]);
            pos = crate::trivia::skip(body, end + 1);
        }
        if sizes.is_empty() {
            return None;
        }
        let end = body[pos..].find(['<', '%']).map_or(body.len(), |i| pos + i);
        let font = body[pos..end].trim_matches(|c: char| c.is_ascii_whitespace());
        if font.is_empty()
            || !font
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/.-_+*[] \t".contains(&c))
        {
            return None;
        }
        widest = widest.max(sizes.len());
        records.push((sizes, font));
        pos = crate::trivia::skip(body, end);
    }
    Some(
        records
            .into_iter()
            .map(|(sizes, font)| {
                if widest + 1 + font.len() <= columns {
                    format!("{sizes:<widest$} {font}")
                } else {
                    format!("{sizes} {font}")
                }
            })
            .collect(),
    )
}
