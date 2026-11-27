//! File encodings, explicit TeX declarations, and lossless write guards.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::{Arc, LazyLock};

use encoding_rs::Encoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    Utf8,
    Latin1,
    Viscii,
    Utf16Le,
    Utf16Be,
    Legacy(&'static Encoding),
}

// VISCII 1.1 (RFC 1456) assigns six C0 bytes to Vietnamese capitals.
// Its other ASCII positions and the ordering of its upper half are fixed.
static VISCII: LazyLock<[char; 256]> = LazyLock::new(|| {
    let mut table = std::array::from_fn(|index| char::from(index as u8));
    for (byte, letter) in [
        (2, 'Ẳ'),
        (5, 'Ẵ'),
        (6, 'Ẫ'),
        (20, 'Ỷ'),
        (25, 'Ỹ'),
        (30, 'Ỵ'),
    ] {
        table[byte] = letter;
    }
    let upper = concat!(
        "ẠẮẰẶẤẦẨẬẼẸẾỀỂỄỆỐ",
        "ỒỔỖỘỢỚỜỞỊỎỌỈỦŨỤỲ",
        "Õắằặấầẩậẽẹếềểễệố",
        "ồổỗỠƠộờởịỰỨỪỬơớƯ",
        "ÀÁÂÃẢĂẳẵÈÉÊẺÌÍĨỳ",
        "ĐứÒÓÔạỷừửÙÚỹỵÝỡư",
        "àáâãảăữẫèéêẻìíĩỉ",
        "đựòóôõỏọụùúũủýợỮ",
    );
    for (slot, letter) in table[128..].iter_mut().zip(upper.chars()) {
        *slot = letter;
    }
    table
});

/// Unicode text for the formatter and the exact bytes used for write guards.
pub(super) struct SourceFile {
    pub text: Arc<String>,
    original: Option<Vec<u8>>,
    codec: Codec,
    fallback: EncodingFallback,
}

#[derive(Clone, Copy)]
enum EncodingFallback {
    Utf8,
    TexDeclaration,
    Default(Codec),
}

impl SourceFile {
    pub fn decode(bytes: Vec<u8>) -> Result<Self, String> {
        Self::decode_with_default(bytes, None)
    }

    pub fn decode_with_default(
        bytes: Vec<u8>,
        input_default: Option<&str>,
    ) -> Result<Self, String> {
        Self::decode_snapshot(bytes, input_default).map_err(|(error, _)| error)
    }

    // An importing XeTeX document may supply a codec for a file that cannot
    // be decoded on its own. Preserve those bytes for every reader in the run.
    pub fn decode_snapshot(
        bytes: Vec<u8>,
        input_default: Option<&str>,
    ) -> Result<Self, (String, Vec<u8>)> {
        let (codec, fallback) = match Self::select_codec(&bytes, input_default) {
            Ok(codec) => codec,
            Err(error) => return Err((error, bytes)),
        };
        if codec == Codec::Utf8 {
            return String::from_utf8(bytes)
                .map(|text| Self { text: Arc::new(text), original: None, codec, fallback })
                .map_err(|error| (
                    format!("invalid UTF-8 at byte {}; declare a different encoding with a header magic comment", error.utf8_error().valid_up_to()),
                    error.into_bytes(),
                ));
        }
        let text = match codec.decode(&bytes).and_then(|text| {
            if codec.encode(&text)? == bytes {
                Ok(text)
            } else {
                Err(format!(
                    "{} input cannot be decoded and re-encoded without changing its bytes",
                    codec.name()
                ))
            }
        }) {
            Ok(text) => text,
            Err(error) => return Err((error, bytes)),
        };
        Ok(Self {
            text: Arc::new(text),
            original: Some(bytes),
            codec,
            fallback,
        })
    }

    fn select_codec(
        bytes: &[u8],
        input_default: Option<&str>,
    ) -> Result<(Codec, EncodingFallback), String> {
        // A UTF-16 BOM lets us read its header. UTF-16 is accepted only when
        // that header explicitly selects it, just like other legacy encodings.
        let bootstrap = if bytes.starts_with(&[0xff, 0xfe]) {
            Some(Codec::Utf16Le)
        } else if bytes.starts_with(&[0xfe, 0xff]) {
            Some(Codec::Utf16Be)
        } else {
            None
        };
        let header = bootstrap.map(|codec| codec.decode(bytes)).transpose()?;
        let declared = declared_codec(header.as_deref().map_or(bytes, str::as_bytes), bootstrap)?;
        let (codec, fallback) = if let Some(codec) = declared {
            (codec, EncodingFallback::Utf8)
        } else if let Some(label) = input_default {
            let codec = Codec::for_label(label, bootstrap)?;
            (codec, EncodingFallback::Default(codec))
        } else if bootstrap.is_none()
            && memchr::memchr(0x1b, bytes).is_some()
            && memchr::memmem::find(bytes, b"\x1b$B").is_some()
            && memchr::memmem::find(bytes, b"\x1b(B").is_some()
            && bytes.is_ascii()
        {
            let codec = Codec::Legacy(encoding_rs::ISO_2022_JP);
            (codec, EncodingFallback::Default(codec))
        } else if bootstrap.is_none() && std::str::from_utf8(bytes).is_err() {
            (
                tex_codec(&String::from_utf8_lossy(bytes)).unwrap_or(Codec::Utf8),
                EncodingFallback::TexDeclaration,
            )
        } else {
            (Codec::Utf8, EncodingFallback::Utf8)
        };
        if bootstrap.is_some_and(|bom| bom != codec)
            || bytes.starts_with(b"\xef\xbb\xbf") && codec != Codec::Utf8
        {
            return Err("encoding magic comment conflicts with the file BOM".into());
        }
        Ok((codec, fallback))
    }

    pub fn original_bytes(&self) -> &[u8] {
        self.original.as_deref().unwrap_or(self.text.as_bytes())
    }

    pub fn is_utf8(&self) -> bool {
        self.codec == Codec::Utf8
    }

    pub fn encode<'a>(&'a self, text: &'a str) -> Result<Cow<'a, [u8]>, String> {
        if text == self.text.as_str() {
            return Ok(Cow::Borrowed(self.original_bytes()));
        }
        let bom = matches!(self.codec, Codec::Utf16Le | Codec::Utf16Be).then_some(self.codec);
        let declared = declared_codec(text.as_bytes(), bom)?;
        let expected = declared.or_else(|| match self.fallback {
            EncodingFallback::Utf8 => Some(Codec::Utf8),
            EncodingFallback::TexDeclaration => tex_codec(text),
            EncodingFallback::Default(codec) => Some(codec),
        });
        if expected.unwrap_or(Codec::Utf8) != self.codec {
            return Err(
                "formatting changed the encoding magic comment; original bytes retained".into(),
            );
        }
        let bytes = self.codec.encode(text)?;
        if !self.is_utf8() && self.codec.decode(&bytes)?.as_str() != text {
            return Err(format!(
                "formatted text cannot be encoded losslessly in {}; original bytes retained",
                self.codec.name()
            ));
        }
        Ok(bytes)
    }

    /// Diagnostics address the original encoded file, including any BOM.
    pub fn original_span(&self, range: Range<usize>) -> Range<usize> {
        if self.codec == Codec::Utf8 {
            return range;
        }
        if self.codec == Codec::Legacy(encoding_rs::ISO_2022_JP) {
            // Encoding a Unicode prefix would append a reset sequence even
            // in the middle of a JIS run. Walk the validated original stream.
            let bytes = self.original_bytes();
            let mut encoded = 0;
            let mut width = 1;
            let mut start = None;
            let end = self
                .text
                .floor_char_boundary(range.end.min(self.text.len()));
            for (at, ch) in self.text.char_indices() {
                while bytes.get(encoded) == Some(&0x1b) {
                    width = if bytes[encoded + 1] == b'$' { 2 } else { 1 };
                    encoded += 3;
                }
                if at + ch.len_utf8() > range.start && start.is_none() {
                    start = Some(encoded);
                }
                if at >= end {
                    return start.unwrap_or(encoded)..encoded;
                }
                encoded += width;
                if start.is_some() && at + ch.len_utf8() == end {
                    return start.unwrap_or(encoded)..encoded;
                }
            }
            return start.unwrap_or(bytes.len())..bytes.len();
        }
        let offset = |mut end: usize| {
            end = end.min(self.text.len());
            while !self.text.is_char_boundary(end) {
                end -= 1;
            }
            self.codec
                .encode(&self.text[..end])
                .expect("validated source prefix is encodable")
                .len()
        };
        offset(range.start)..offset(range.end)
    }
}

fn declared_codec(bytes: &[u8], bom: Option<Codec>) -> Result<Option<Codec>, String> {
    let mut codec = None;
    for declaration in magic_encodings(bytes)? {
        let next = Codec::for_label(&declaration, bom)?;
        if codec.is_some_and(|previous| previous != next) {
            return Err("conflicting encoding magic comments in the file header".into());
        }
        codec = Some(next);
    }
    Ok(codec)
}

/// Only literal, unconditional package loads in the opening preamble select
/// a fallback. A quoted command in a macro, example or comment is inert.
fn tex_codec(source: &str) -> Option<Codec> {
    let (selected, cjk) = preamble_encodings(source)?;
    if cjk && source.contains(r"\begin{CJK") {
        return cjk_document_codec(source)
            .filter(|codec| selected.is_none_or(|other| other == *codec));
    }
    selected
}

/// inputenc remains active when TeX opens a child file. A CJK environment
/// and an editor's file-local encoding have different lifetimes.
pub(super) fn inputenc_at_loader(prefix: &str) -> Option<&'static str> {
    if !(prefix.contains("inputenc") || prefix.contains("{eskd")) {
        return None;
    }
    preamble_encodings(prefix)?.0.map(Codec::name)
}

fn preamble_encodings(source: &str) -> Option<(Option<Codec>, bool)> {
    static COMMENT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"%[^\r\n]*(?:\r\n?|\n|\z)").expect("TeX comment"));
    static LOAD: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?x)\A (?:\s|%[^\r\n]*(?:[\r\n]|\z))* \\(documentclass|usepackage|RequirePackage)
        (?:\s|%[^\r\n]*[\r\n])* (?:\[((?:[^\]\\%]|%[^\r\n]*[\r\n])*)\](?:\s|%[^\r\n]*[\r\n])*)?
        \{([A-Za-z0-9.,_\x20-]+)\}(?:\s*\[[0-9/.-]+\])?",
        )
        .expect("package declaration")
    });
    // Common literal declarations before inputenc. Keep this prefix grammar
    // closed so a package load inside a definition or conditional stays inert.
    static SETUP: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?x)\A (?:\s|%[^\r\n]*(?:[\r\n]|\z))* \\ (?:
            (?:listfiles|makeindex)\b
            | (?P<companion>(?:Start|Stop)ShownPreambleCommands)\b
            | PassOptionsToClass\s*\{[A-Za-z0-9.,=_-]+\}\s*\{[a-zA-Z0-9._-]+\}
            | renewcommand\s*\{\\familydefault\}\s*\{\\(?:sfdefault|rmdefault|ttdefault)\}
            | (?:newcommand|providecommand)\*?\s*\{\\[A-Za-z]+\}\s*\{[A-Za-z0-9._-]+\}
            | (?:renewcommand\s*\{\\(?:rmdefault|sfdefault|ttdefault)\}
                | (?:e?def)\s*\\my[A-Za-z]+)
                \s*\{[A-Za-z0-9._-]+\}
            | (?:ProvidesFile|NeedsTeXFormat)\s*\{[A-Za-z0-9._-]+\}
                (?:(?:\s|%[^\r\n]*(?:[\r\n]|\z))*\[[^\]\\%{}]*\])?
            | pagestyle\s*\{[A-Za-z]+\}
            | setcounter\s*\{[A-Za-z]+\}\s*\{-?[0-9]+\}
            | (?:setlength|addtolength)\s*(?:\\[A-Za-z]+|\{\\[A-Za-z]+\})
                \s*\{-?[0-9.]+[a-z]{2}\}
        )",
        )
        .expect("literal preamble setup")
    });
    if source.contains(r"\inputencoding") || source.contains(r"\XeTeXinputencoding") {
        return None;
    }
    let mut rest = source;
    let mut selected = None;
    let mut cjk = false;
    let mut companion_markers = false;
    loop {
        let Some(load) = LOAD.captures(rest) else {
            // ttctexa defines these two example-preamble markers as \relax.
            if let Some(setup) = SETUP.captures(rest)
                && (companion_markers || setup.name("companion").is_none())
            {
                rest = &rest[setup.get(0)?.end()..];
                continue;
            }
            break;
        };
        if &load[1] == "documentclass" {
            companion_markers = &load[3] == "ttctexa";
            if matches!(&load[3], "eskdtext" | "eskdtab" | "eskdgraph")
                && let Some(options) = load.get(2)
            {
                for label in COMMENT
                    .replace_all(options.as_str(), "")
                    .split(',')
                    .map(str::trim)
                {
                    if matches!(
                        label,
                        "koi8-r" | "koi8-u" | "cp1251" | "iso8859-5" | "cp866" | "utf8" | "utf-8"
                    ) {
                        let codec = Codec::for_label(label, None).ok()?;
                        if selected.replace(codec).is_some_and(|first| first != codec) {
                            return None;
                        }
                    }
                }
            }
        } else {
            for package in load[3].split(',').map(str::trim) {
                if package == "inputenc" {
                    let label = COMMENT.replace_all(load.get(2)?.as_str(), "");
                    let codec = Codec::for_label(label.trim(), None).ok()?;
                    if selected.is_some_and(|previous| previous != codec) {
                        return None;
                    }
                    selected = Some(codec);
                }
                cjk |= package == "CJK";
            }
        }
        rest = &rest[load.get(0)?.end()..];
    }
    Some((selected, cjk))
}

/// A CJK encoding is local to its environment. Use one file codec only when
/// all non-ASCII data lies inside matching literal CJK environments.
fn cjk_document_codec(source: &str) -> Option<Codec> {
    static HEADER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\A\s*(?:\[[a-zA-Z0-9-]+\]\s*)?\{([a-zA-Z0-9_-]+)\}")
            .expect("CJK encoding argument")
    });
    let config = tift::Config::default();
    let syntax = tift::syntax::compile(&config.syntax).ok()?;
    let mut selected = None;
    let mut active = None;
    let mut outside = 0;
    for token in tift::syntax::lex(source, &config, &syntax).ok()? {
        if !matches!(
            token.kind,
            tift::syntax::TokenKind::BeginEnvironment | tift::syntax::TokenKind::EndEnvironment
        ) {
            continue;
        }
        let Some((name @ ("CJK" | "CJK*"), begin, end)) = token.environment_at(source) else {
            continue;
        };
        if begin {
            if active.is_some() || !source[outside..token.span.start].is_ascii() {
                return None;
            }
            let header = HEADER.captures(&source[end..])?;
            let codec = Codec::for_label(&header[1], None).ok()?;
            if selected.is_some_and(|previous| previous != codec) {
                return None;
            }
            selected = Some(codec);
            active = Some(name.to_owned());
        } else {
            if active.take().as_deref() != Some(name) {
                return None;
            }
            outside = end;
        }
    }
    selected.filter(|_| active.is_none() && source[outside..].is_ascii())
}

impl Codec {
    fn for_label(label: &str, bom: Option<Self>) -> Result<Self, String> {
        let normalized = label.to_ascii_lowercase().replace(['-', '_', ' '], "");
        let codec = match normalized.as_str() {
            "utf8" | "utf8x" => Self::Utf8,
            "latin1" | "isolatin1" | "iso88591" | "isoir100" => Self::Latin1,
            "viscii" | "csviscii" | "viscii1.11" => Self::Viscii,
            "utf16" => bom.ok_or("UTF-16 requires a BOM and an encoding magic comment")?,
            "utf16le" => Self::Utf16Le,
            "utf16be" => Self::Utf16Be,
            _ => {
                let alias = match normalized.as_str() {
                    "ansinew" | "windowslatin1" => "windows-1252",
                    "latin2" | "isolatin2" => "iso-8859-2",
                    "latin3" => "iso-8859-3",
                    "latin4" => "iso-8859-4",
                    "latin7" => "iso-8859-13",
                    "applemac" => "macintosh",
                    "latin5" => "iso-8859-9",
                    "latin9" | "isolatin9" => "iso-8859-15",
                    "latin10" => "iso-8859-16",
                    "macosroman" => "macintosh",
                    "windowscyrillic" => "windows-1251",
                    "eucchina" => "gbk",
                    "eucjapan" => "euc-jp",
                    "euckorea" => "euc-kr",
                    "thaitis620" => "tis-620",
                    _ => label,
                };
                let encoding = Encoding::for_label(alias.as_bytes())
                    .filter(|encoding| {
                        ![
                            encoding_rs::REPLACEMENT,
                            encoding_rs::UTF_16LE,
                            encoding_rs::UTF_16BE,
                        ]
                        .contains(encoding)
                    })
                    .ok_or_else(|| format!("unsupported encoding in magic comment: {label}"))?;
                if encoding == encoding_rs::UTF_8 {
                    Self::Utf8
                } else {
                    Self::Legacy(encoding)
                }
            }
        };
        Ok(codec)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Latin1 => "ISO-8859-1",
            Self::Viscii => "VISCII",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
            Self::Legacy(encoding) => encoding.name(),
        }
    }

    fn decode(self, bytes: &[u8]) -> Result<String, String> {
        let invalid = || format!("invalid {} input; original bytes retained", self.name());
        match self {
            Self::Utf8 => String::from_utf8(bytes.to_vec()).map_err(|_| invalid()),
            Self::Latin1 => Ok(bytes.iter().map(|&byte| char::from(byte)).collect()),
            Self::Viscii => Ok(bytes
                .iter()
                .map(|&byte| VISCII[usize::from(byte)])
                .collect()),
            Self::Utf16Le | Self::Utf16Be => {
                if !bytes.len().is_multiple_of(2) {
                    return Err(invalid());
                }
                let words = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| {
                        if self == Self::Utf16Le {
                            u16::from_le_bytes([pair[0], pair[1]])
                        } else {
                            u16::from_be_bytes([pair[0], pair[1]])
                        }
                    })
                    .collect::<Vec<_>>();
                String::from_utf16(&words).map_err(|_| invalid())
            }
            Self::Legacy(encoding) => encoding
                .decode_without_bom_handling_and_without_replacement(bytes)
                .map(Cow::into_owned)
                .ok_or_else(invalid),
        }
    }

    fn encode(self, text: &str) -> Result<Cow<'_, [u8]>, String> {
        let unrepresentable = || {
            format!(
                "formatted text cannot be represented in {}; original bytes retained",
                self.name()
            )
        };
        match self {
            Self::Utf8 => Ok(Cow::Borrowed(text.as_bytes())),
            Self::Latin1 => text
                .chars()
                .map(|character| u8::try_from(character as u32).map_err(|_| unrepresentable()))
                .collect::<Result<Vec<_>, _>>()
                .map(Cow::Owned),
            Self::Viscii => text
                .chars()
                .map(|ch| {
                    if ch.is_ascii() && VISCII[ch as usize] == ch {
                        return Ok(ch as u8);
                    }
                    VISCII
                        .iter()
                        .position(|&value| value == ch)
                        .map(|byte| byte as u8)
                        .ok_or_else(unrepresentable)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Cow::Owned),
            Self::Utf16Le | Self::Utf16Be => Ok(Cow::Owned(
                text.encode_utf16()
                    .flat_map(|word| {
                        if self == Self::Utf16Le {
                            word.to_le_bytes()
                        } else {
                            word.to_be_bytes()
                        }
                    })
                    .collect(),
            )),
            Self::Legacy(encoding) => {
                let (encoded, _, errors) = encoding.encode(text);
                if errors {
                    Err(unrepresentable())
                } else {
                    Ok(encoded)
                }
            }
        }
    }
}

fn magic_encodings(bytes: &[u8]) -> Result<Vec<String>, String> {
    fn emacs_codec(label: &str) -> String {
        let label = label.trim().to_ascii_lowercase();
        ["-unix", "-dos", "-mac"]
            .iter()
            .find_map(|suffix| label.strip_suffix(suffix))
            .unwrap_or(&label)
            .to_owned()
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut labels = Vec::new();
    // Emacs recognizes a Local Variables block at the end of the file.
    // Every line must use the same comment prefix as the opening marker.
    let tail = &bytes[bytes.len().saturating_sub(3000)..];
    if memchr::memmem::rfind(tail, b"Local Variables:").is_some() {
        let mut lines = tail.rsplit(|&ch| ch == b'\n').map(<[u8]>::trim_ascii);
        if let Some(last) = lines.find(|line| !line.is_empty())
            && let Some(prefix) = last.strip_suffix(b"End:")
            && prefix.trim_ascii().starts_with(b"%")
            && prefix
                .iter()
                .all(|ch| *ch == b'%' || ch.is_ascii_whitespace())
        {
            let mut block = Vec::new();
            for line in lines {
                let Some(field) = line.strip_prefix(prefix) else {
                    break;
                };
                if field == b"Local Variables:" {
                    labels.extend(block);
                    break;
                }
                if let Some(label) = field.strip_prefix(b"coding:")
                    && let Ok(label) = std::str::from_utf8(label.trim_ascii())
                {
                    block.push(emacs_codec(label));
                }
            }
        }
    }
    let line_ends = memchr::memchr2_iter(b'\r', b'\n', bytes)
        .filter(|&end| bytes[end] == b'\n' || bytes.get(end + 1) != Some(&b'\n'))
        .chain(std::iter::once(bytes.len()));
    let mut start = 0;
    for (line_number, end) in line_ends.take(20).enumerate() {
        let line = bytes[start..end].trim_ascii();
        start = end + 1;
        if line.is_empty() {
            continue;
        }
        // A first-line Emacs modeline may follow TeX code on the same line.
        let comment = line.strip_prefix(b"%").or_else(|| {
            (line_number == 0).then_some(())?;
            let at = memchr::memchr(b'%', line)?;
            let tail = line[at + 1..].trim_ascii();
            (tail.starts_with(b"-*-") && tail.ends_with(b"-*-")).then_some(tail)
        });
        let Some(comment) = comment else {
            break;
        };
        let ascii_end = comment
            .iter()
            .position(|byte| !byte.is_ascii())
            .unwrap_or(comment.len());
        let ascii_only = ascii_end == comment.len();
        let comment = std::str::from_utf8(&comment[..ascii_end])
            .expect("ASCII header")
            .trim();
        let lower = comment.to_ascii_lowercase();
        // Some TeX manuals mark the source codec inside a bordered comment.
        let banner = lower.trim_matches('%').trim();
        if ascii_only
            && let Some(rest) = banner.strip_prefix("this file is encoded in ")
            && let Some(label) = rest.split_ascii_whitespace().next()
            && label.starts_with("iso-8859-")
            && label["iso-8859-".len()..]
                .bytes()
                .all(|ch| ch.is_ascii_digit())
        {
            labels.push(label.to_owned());
            continue;
        }
        if ascii_only
            && let Some(number) = banner.strip_prefix("encodage iso 8859-")
            && !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
        {
            labels.push(format!("iso-8859-{number}"));
            continue;
        }
        if line_number < 2
            && ascii_only
            && let Some(modeline) = lower
                .strip_prefix("-*-")
                .and_then(|s| s.strip_suffix("-*-"))
        {
            for field in modeline.split(';') {
                if let Some(label) = field.trim().strip_prefix("coding:") {
                    labels.push(emacs_codec(label));
                }
            }
            continue;
        }
        let setting = if let Some(tex) = lower.strip_prefix('!') {
            let tex = tex.trim_start();
            let Some(setting) = tex
                .strip_prefix("tex")
                .filter(|rest| rest.starts_with(char::is_whitespace))
            else {
                continue;
            };
            setting.trim_start()
        } else {
            lower.as_str()
        };
        let Some(value) = setting.strip_prefix("encoding") else {
            continue;
        };
        let Some(value) = value.trim_start().strip_prefix(['=', ':']) else {
            continue;
        };
        let value = value.trim();
        let label = value.strip_suffix(" unicode").unwrap_or(value).trim();
        if !ascii_only
            || label.is_empty()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_. ".contains(&byte))
        {
            return Err("invalid encoding magic comment in the file header".into());
        }
        labels.push(label.to_owned());
    }
    Ok(labels)
}
