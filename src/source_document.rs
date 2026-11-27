//! Immutable source text and source coordinates shared by analysis layers.

use std::ops::{Deref, Range};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub(crate) struct SourceDocument {
    text: Arc<str>,
    newlines: Arc<OnceLock<Vec<usize>>>,
    translated: Arc<OnceLock<Option<TranslatedSource>>>,
}

#[derive(Debug)]
struct TranslatedSource {
    text: Arc<str>,
    segments: Arc<[SourceMapSegment]>,
    translated_ranges: Arc<[Range<usize>]>,
}

#[derive(Debug)]
struct SourceMapSegment {
    analysis: Range<usize>,
    source: Range<usize>,
}

/// One changed interval between byte-identical prefixes and suffixes.
#[derive(Debug, Clone)]
pub(crate) struct SourceChange {
    pub(crate) old: Range<usize>,
    pub(crate) new: Range<usize>,
}

impl SourceChange {
    pub(crate) fn between(old: &str, new: &str) -> Self {
        let common = old
            .as_bytes()
            .iter()
            .zip(new.as_bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let first = old.floor_char_boundary(common);
        let suffix = old.as_bytes()[first..]
            .iter()
            .rev()
            .zip(new.as_bytes()[first..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let end = old.ceil_char_boundary(old.len() - suffix);
        Self {
            old: first..end,
            new: first..new.len() - (old.len() - end),
        }
    }

    /// Caller has established that no stored boundary lies inside this text edit.
    pub(crate) fn position(&self, at: usize) -> usize {
        if at < self.old.end {
            at
        } else {
            self.new.end + (at - self.old.end)
        }
    }

    pub(crate) fn project(&self, range: &mut Range<usize>) {
        *range = self.position(range.start)..self.position(range.end);
    }
}

impl SourceDocument {
    pub(crate) fn new(source: &str) -> Self {
        Self {
            text: Arc::from(source),
            newlines: Arc::default(),
            translated: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) fn analysis_text(&self) -> &str {
        self.translated_source()
            .map_or(self.as_str(), |translated| translated.text.as_ref())
    }

    /// A literal reader consumes the original bytes, including a delimiter
    /// immediately following a pair of carets.
    pub(crate) fn with_literal_sequences(mut self, literals: &[Range<usize>]) -> Self {
        if !literals.is_empty() {
            self.translated = Arc::new(OnceLock::from(translate_ascii_sequences(
                self.as_str(),
                literals,
            )));
        }
        self
    }

    pub(crate) fn translated_ranges(&self) -> &[Range<usize>] {
        self.translated_source()
            .map_or(&[], |translated| translated.translated_ranges.as_ref())
    }

    pub(crate) fn to_source_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        let analysis = self.analysis_text();
        if range.start > range.end
            || range.end > analysis.len()
            || !analysis.is_char_boundary(range.start)
            || !analysis.is_char_boundary(range.end)
        {
            return None;
        }
        Some(self.to_source_offset(range.start)?..self.to_source_offset(range.end)?)
    }

    /// Map a raw source range outward so no source bytes are omitted.
    pub(crate) fn to_analysis_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.start > range.end
            || range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return None;
        }
        let Some(translated) = self.translated_source() else {
            return Some(range);
        };
        Some(
            map_source_offset(translated, range.start, false)?
                ..map_source_offset(translated, range.end, true)?,
        )
    }

    pub(crate) fn to_source_offset(&self, offset: usize) -> Option<usize> {
        let analysis = self.analysis_text();
        if offset > analysis.len() || !analysis.is_char_boundary(offset) {
            return None;
        }
        let Some(translated) = self.translated_source() else {
            return Some(offset);
        };
        map_analysis_offset(translated, offset)
    }

    pub(crate) fn matches(&self, source: &str) -> bool {
        if self.text.as_ptr() == source.as_ptr() && self.text.len() == source.len() {
            return true;
        }
        self.text.as_ref() == source
    }

    pub(crate) fn newlines(&self) -> &[usize] {
        self.newlines
            .get_or_init(|| memchr::memchr_iter(b'\n', self.text.as_bytes()).collect())
    }

    pub(crate) fn line_range(&self, line: usize) -> Option<Range<usize>> {
        let newlines = self.newlines();
        let start = match line.checked_sub(1) {
            Some(previous) => newlines.get(previous)? + 1,
            None => 0,
        };
        (start < self.text.len())
            .then(|| start..newlines.get(line).map_or(self.text.len(), |at| at + 1))
    }

    pub(crate) fn lines(&self) -> impl Iterator<Item = (usize, &str)> {
        (0..).map_while(|line| {
            self.line_range(line)
                .map(|range| (range.start, &self.text[range]))
        })
    }

    fn translated_source(&self) -> Option<&TranslatedSource> {
        self.translated
            .get_or_init(|| translate_ascii_sequences(self.as_str(), &[]))
            .as_ref()
    }
}

fn map_analysis_offset(translated: &TranslatedSource, offset: usize) -> Option<usize> {
    let index = translated
        .segments
        .partition_point(|segment| segment.analysis.end < offset);
    let segment = translated.segments.get(index)?;
    if offset < segment.analysis.start || offset > segment.analysis.end {
        return None;
    }
    let analysis_len = segment.analysis.len();
    let source_len = segment.source.len();
    if analysis_len == source_len {
        Some(segment.source.start + offset - segment.analysis.start)
    } else if offset == segment.analysis.start {
        Some(segment.source.start)
    } else if offset == segment.analysis.end {
        Some(segment.source.end)
    } else {
        None
    }
}

fn map_source_offset(
    translated: &TranslatedSource,
    offset: usize,
    round_up: bool,
) -> Option<usize> {
    let index = translated
        .segments
        .partition_point(|segment| segment.source.end < offset);
    let segment = translated.segments.get(index)?;
    if offset < segment.source.start || offset > segment.source.end {
        return None;
    }
    let analysis_len = segment.analysis.len();
    let source_len = segment.source.len();
    if analysis_len == source_len {
        Some(segment.analysis.start + offset - segment.source.start)
    } else if offset == segment.source.start {
        Some(segment.analysis.start)
    } else if offset == segment.source.end || round_up {
        Some(segment.analysis.end)
    } else {
        Some(segment.analysis.start)
    }
}

fn translate_ascii_sequences(source: &str, literals: &[Range<usize>]) -> Option<TranslatedSource> {
    if !source.contains("^^") {
        return None;
    }

    let mut analysis = String::with_capacity(source.len());
    let mut segments = Vec::new();
    let mut translated_ranges = Vec::new();
    let mut cursor = 0;
    let mut literal = literals.iter().peekable();
    while cursor < source.len() {
        while literal.peek().is_some_and(|range| range.end <= cursor) {
            literal.next();
        }
        if let Some(range) = literal.peek().filter(|range| range.start <= cursor) {
            append_source_segment(source, cursor..range.end, &mut analysis, &mut segments);
            cursor = range.end;
            continue;
        }
        if let Some((range, value)) = character_translation_at(source, cursor)
            && value <= 0x7f
        {
            let start = analysis.len();
            analysis.push(value as char);
            let end = analysis.len();
            segments.push(SourceMapSegment {
                analysis: start..end,
                source: range.clone(),
            });
            translated_ranges.push(start..end);
            cursor = range.end;
            continue;
        }

        let next_sequence = source[cursor..]
            .find("^^")
            .map_or(source.len(), |offset| cursor + offset);
        if next_sequence > cursor {
            append_source_segment(source, cursor..next_sequence, &mut analysis, &mut segments);
            cursor = next_sequence;
            continue;
        }

        let end = character_translation_at(source, cursor).map_or_else(
            || {
                cursor
                    + source[cursor..]
                        .chars()
                        .next()
                        .expect("nonempty source")
                        .len_utf8()
            },
            |(range, _)| range.end,
        );
        append_source_segment(source, cursor..end, &mut analysis, &mut segments);
        cursor = end;
    }

    if translated_ranges.is_empty() {
        return None;
    }

    Some(TranslatedSource {
        text: analysis.into(),
        segments: segments.into(),
        translated_ranges: translated_ranges.into(),
    })
}

fn append_source_segment(
    source: &str,
    range: Range<usize>,
    analysis: &mut String,
    segments: &mut Vec<SourceMapSegment>,
) {
    if range.is_empty() {
        return;
    }
    let analysis_start = analysis.len();
    analysis.push_str(&source[range.clone()]);
    let analysis_range = analysis_start..analysis.len();
    if let Some(previous) = segments.last_mut().filter(|segment| {
        segment.analysis.end == analysis_range.start
            && segment.source.end == range.start
            && segment.analysis.len() == segment.source.len()
    }) {
        previous.analysis.end = analysis_range.end;
        previous.source.end = range.end;
    } else {
        segments.push(SourceMapSegment {
            analysis: analysis_range,
            source: range,
        });
    }
}

fn character_translation_at(source: &str, start: usize) -> Option<(Range<usize>, u8)> {
    let sequence = source.get(start..)?.strip_prefix("^^")?;
    let bytes = sequence.as_bytes();
    let first = *bytes.first()?;
    if let (Some(&second), Some(high)) = (bytes.get(1), hex_value(first))
        && let Some(low) = hex_value(second)
    {
        return Some((start..start + 4, high * 16 + low));
    }
    if !first.is_ascii() {
        return None;
    }
    let translated = if first < 64 { first + 64 } else { first - 64 };
    Some((start..start + 3, translated))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl Deref for SourceDocument {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}
