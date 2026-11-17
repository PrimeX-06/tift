//! Literal headers and body roles from graphics, animation and science packages.
//!
//! Header settings are data, including delayed begin/end hooks. Raw readers
//! retain their package-specific end markers. Declarations and explicit syntax
//! can replace the built-in grammar at each call site.

use std::collections::{BTreeMap, HashSet};

use crate::config::CommandRule;

/// Record changed visual/science environment and command bindings, preserving
/// TeX scope recovery. The built-in command and environment names are disjoint.
#[derive(Default)]
pub(crate) struct Overrides {
    active: HashSet<String>,
    scopes: Vec<Vec<(String, bool)>>,
}

impl Overrides {
    pub(crate) fn enter(&mut self) {
        self.scopes.push(Vec::new());
    }

    pub(crate) fn leave(&mut self) {
        if let Some(changes) = self.scopes.pop() {
            for (name, was_active) in changes {
                if was_active {
                    self.active.insert(name);
                } else {
                    self.active.remove(&name);
                }
            }
        }
    }

    pub(crate) fn assign(&mut self, name: &str, global: bool) {
        if global {
            for changes in &mut self.scopes {
                changes.retain(|(key, _)| key != name);
            }
        } else if let Some(changes) = self.scopes.last_mut()
            && !changes.iter().any(|(key, _)| key == name)
        {
            changes.push((name.to_owned(), self.active.contains(name)));
        }
        self.active.insert(name.to_owned());
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.active.contains(name)
    }

    pub(crate) fn restore_default(&mut self, name: &str) {
        if !self.active.contains(name) {
            return;
        }
        if let Some(changes) = self.scopes.last_mut()
            && !changes.iter().any(|(key, _)| key == name)
        {
            changes.push((name.to_owned(), true));
        }
        self.active.remove(name);
    }
}

pub(crate) fn environment_for_command(name: &str) -> Option<&str> {
    if header_signature(name).is_some() {
        Some(name)
    } else {
        name.strip_prefix("end")
            .filter(|name| header_signature(name).is_some())
    }
}

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &["reactions", "reactions*", "psmatrix"];

/// Names with a protected data prefix; use the unstarred command as its binding
/// key. Environment keys remain exact, including their separately defined stars.
pub(crate) fn is_metadata_command(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "switchocg"
            | "showocg"
            | "hideocg"
            | "actionsocg"
            | "toggleocgs"
            | "showocgs"
            | "hideocgs"
            | "setocgs"
            | "animategraphics"
            | "newframe"
            | "multiframe"
    )
}

/// Public controls own only their declared arguments. In particular, the
/// braced first token of the next animation frame is not a newframe argument.
pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // ocgx2.sty:788--801,1007--1025,1245--1274 (v0.60).
            ("switchocg", 2, true),
            ("showocg", 2, true),
            ("hideocg", 2, true),
            ("actionsocg", 4, true),
            ("toggleocgs", 2, true),
            ("showocgs", 2, true),
            ("hideocgs", 2, true),
            ("setocgs", 4, true),
            ("ocglinkprotect", 1, false),
            // animate.sty:1562--1567,2240--2285,2369--2400 (2024/10/14).
            ("animategraphics", 4, true),
            ("newframe", 0, true),
            ("multiframe", 3, false),
            ("multiframebreak", 0, false),
        ],
    );
}

fn mandatory_argument_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => crate::syntax::balanced_end(source, start, b'{', b'}'),
        b'\\' => crate::syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' => None,
        _ => source[start..].chars().next().map(|c| start + c.len_utf8()),
    }
}

/// Complete invocation ranges retain the printed argument after an opaque
/// metadata prefix and bound ownership before the next document group.
pub(crate) fn command_signature(source: &str, name: &str, end: usize) -> Option<&'static str> {
    let separated_star = !name.ends_with('*')
        && source.as_bytes().get(crate::trivia::skip(source, end)) == Some(&b'*');
    Some(match name.trim_end_matches('*') {
        "switchocg" | "showocg" | "hideocg" if separated_star => "momm",
        "switchocg" | "showocg" | "hideocg" => "omm",
        "actionsocg" if separated_star => "mommmm",
        "actionsocg" => "ommmm",
        "toggleocgs" | "showocgs" | "hideocgs" => "omm",
        "setocgs" => "ommmm",
        "animategraphics" => "ommmm",
        "newframe" if separated_star => "mo",
        "newframe" => "o",
        "multiframe" => "mmm",
        _ => return None,
    })
}

/// Protect identifiers, frame numbers, variable declarations and deferred
/// hooks as data. The final OCG link or multiframe body remains ordinary TeX.
/// The caller gives explicit syntax and source redefinitions precedence.
pub(crate) fn command_header_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let (metadata, optional, accepts_star, content) = match name.trim_end_matches('*') {
        "switchocg" | "showocg" | "hideocg" => (1, true, true, true),
        "actionsocg" => (3, true, true, true),
        "toggleocgs" | "showocgs" | "hideocgs" => (1, true, false, true),
        "setocgs" => (3, true, false, true),
        "animategraphics" => (4, true, false, false),
        "newframe" => (0, true, true, false),
        "multiframe" => (2, false, false, true),
        _ => return None,
    };
    let mut cursor = end;
    if accepts_star && !name.ends_with('*') {
        let start = crate::trivia::skip(source, cursor);
        if source.as_bytes().get(start) == Some(&b'*') {
            cursor = start + 1;
        }
    }
    if optional {
        let start = crate::trivia::skip(source, cursor);
        if source.as_bytes().get(start) == Some(&b'[') {
            cursor = crate::syntax::balanced_end(source, start, b'[', b']')?;
        }
    }
    for _ in 0..metadata {
        cursor = mandatory_argument_end(source, crate::trivia::skip(source, cursor))?;
    }
    if content {
        mandatory_argument_end(source, crate::trivia::skip(source, cursor))?;
    }
    (cursor > end).then_some(cursor)
}

/// ocg-p.sty reads the column preamble before database and listings options.
/// Its listings-read table body retains the package's verbatim protection.
pub(crate) fn column_spec_slot(name: &str) -> Option<usize> {
    (name == "ocgtabular").then_some(1)
}

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // ocgx2.sty:180,254; ocg-p.sty:64,180; animate.sty:2101.
        "ocg" | "ocgtabular" => "ommm",
        "ocmd" | "animateinline" => "om",
        // overpic.sty:97,105: picture options, then the image filename.
        "overpic" | "Overpic" => "om",
        // rotating.sty:73,84,95,166,227.
        "sideways" => "",
        "turn" | "rotate" => "m",
        "sidewaysfigure" | "sidewaysfigure*" | "sidewaystable" | "sidewaystable*" => "o",
        // wrapfig.sty:40--60 adds a float type before the wrapfigure header.
        "wrapfloat" => "momom",
        // floatrow.sty:845,889,892.
        "floatrow" | "subfloatrow" | "subfloatrow*" => "o",
        // floatflt.sty:52,96: optional side and width/table object.
        "floatingfigure" | "floatingtable" => "om",
        // trimclip.sty:86--127: one trimming/margin argument, collected body.
        "clipbox" | "clipbox*" | "trimbox" | "trimbox*" | "marginbox" | "marginbox*" => "m",
        // adjustbox.sty:732--881,1366--1615,1829--1971.
        "centerbox"
        | "leftalignbox"
        | "rightalignbox"
        | "outersidebox"
        | "innersidebox"
        | "pagecenterbox"
        | "textareacenterbox"
        | "pageleftalignbox"
        | "pagerightalignbox"
        | "textarealeftalignbox"
        | "textarearightalignbox" => "o",
        "bgcolorbox" | "fgimagebox" | "fgimagebox*" | "bgimagebox" | "bgimagebox*"
        | "foregroundbox" | "foregroundbox*" | "backgroundbox" | "backgroundbox*" | "lapbox"
        | "rndframebox" | "rndfbox" => "om",
        "bgcolorbox*" | "rndcornersbox" | "adjnofloat" => "m",
        "stackbox" => "ooo",
        "splitbox" => "mm",
        "pagebreakbox" => "",
        // asymptote.sty and gnuplottex.sty use raw line readers.
        "asy" | "gnuplot" => "o",
        "asydef" => "",
        "gnuplotverbatimwrite" => "m",
        // mfpic.tex:1344--1346: two scales and four coordinate bounds.
        "mfpic" => "oommmm",
        // PSTricks picture bounds and psgraph's arrows, origin/bounds and size.
        "pspicture" | "pspicture*" => "opp",
        "psgraph" | "psgraph*" => "ogpppmm",
        "psclip" => "om",
        "psmatrix" => "o",
        "TeXtoEPS" | "psfrags" => "",
        // pst-pdf.sty:141,378,458 stores ordinary TeX/PostScript content.
        "postscript" => "o",
        // pgfcorescopes.code.tex:206--226: no arguments or four legacy bounds.
        "pgfpicture" => "gggg",
        // chemmacros.sty:4162--4243 uses !O{}: whitespace starts the body.
        "reaction" | "reaction*" | "reactions" | "reactions*" => "O",
        // chemmacros.sty:4896,5360: float placement and experimental options.
        "scheme" | "scheme*" | "experimental" => "o",
        // circuitikz.sty:400--403 and tikzlibraryfeynman.code.tex:149--179.
        "circuitikz" | "feynman" => "o",
        // feynmp.sty / feynmf.sty: file, graph, subgraph and loop dimensions.
        "fmffile" | "fmfshrink" => "m",
        "fmfgraph" | "fmfgraph*" | "fmfchar" | "fmfchar*" => "p",
        "fmfsubgraph" => "pp",
        "fmffor" => "mmmm",
        "fmfgroup" => "",
        _ => return None,
    })
}

pub(crate) fn invocation_signature(source: &str, start: usize, name: &str) -> Option<&'static str> {
    if name == "pgfpicture" {
        let next = crate::trivia::skip(source, start);
        Some(if source.as_bytes().get(next) == Some(&b'{') {
            "mmmm"
        } else {
            ""
        })
    } else {
        header_signature(name)
    }
}

pub(crate) fn is_verbatim(name: &str) -> bool {
    matches!(
        name,
        "asy" | "asydef" | "gnuplot" | "gnuplotverbatimwrite" | "ocgtabular"
    )
}

pub(crate) fn has_exact_line_end(name: &str) -> bool {
    matches!(name, "asy" | "asydef")
}

pub(crate) fn ignores_end_line_tail(name: &str) -> bool {
    matches!(name, "gnuplot" | "gnuplotverbatimwrite")
}

/// Asymptote compares the complete line with its literal end spelling.
/// TeX strips trailing ASCII spaces before tokenization; indentation remains.
pub(crate) fn exact_line_end(source: &str, begin_end: usize, name: &str) -> Option<usize> {
    let terminator = format!("\\end{{{name}}}");
    let mut offset = source[begin_end..]
        .find('\n')
        .map_or(source.len(), |relative| begin_end + relative + 1);
    for line in source[offset..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n', ' ']) == terminator {
            return Some(offset + line.len());
        }
        offset += line.len();
    }
    None
}

pub(crate) fn is_display_math(name: &str) -> bool {
    matches!(name, "reaction" | "reaction*" | "reactions" | "reactions*")
}

pub(crate) fn is_box(name: &str) -> bool {
    matches!(
        name,
        "animateinline"
            | "overpic"
            | "Overpic"
            | "sideways"
            | "turn"
            | "rotate"
            | "clipbox"
            | "clipbox*"
            | "trimbox"
            | "trimbox*"
            | "marginbox"
            | "marginbox*"
            | "mfpic"
            | "circuitikz"
            | "feynman"
            | "fmfgraph"
            | "fmfgraph*"
            | "fmfchar"
            | "fmfchar*"
            | "fmfsubgraph"
            | "pspicture"
            | "pspicture*"
            | "psgraph"
            | "psgraph*"
            | "psclip"
            | "psmatrix"
            | "TeXtoEPS"
            | "psfrags"
            | "postscript"
            | "pgfpicture"
            | "centerbox"
            | "leftalignbox"
            | "rightalignbox"
            | "outersidebox"
            | "innersidebox"
            | "pagecenterbox"
            | "textareacenterbox"
            | "pageleftalignbox"
            | "pagerightalignbox"
            | "textarealeftalignbox"
            | "textarearightalignbox"
            | "bgcolorbox"
            | "bgcolorbox*"
            | "fgimagebox"
            | "fgimagebox*"
            | "bgimagebox"
            | "bgimagebox*"
            | "foregroundbox"
            | "foregroundbox*"
            | "backgroundbox"
            | "backgroundbox*"
            | "lapbox"
            | "stackbox"
            | "splitbox"
            | "pagebreakbox"
            | "rndcornersbox"
            | "rndframebox"
            | "rndfbox"
    )
}

pub(crate) fn is_block(name: &str) -> bool {
    header_signature(name).is_some()
        && !is_box(name)
        && !is_verbatim(name)
        && !matches!(name, "ocg" | "ocmd")
}

pub(crate) fn preserves_tight_whitespace(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn is_non_prose(name: &str) -> bool {
    is_display_math(name)
        || is_verbatim(name)
        || matches!(
            name,
            "animateinline"
                | "overpic"
                | "Overpic"
                | "mfpic"
                | "circuitikz"
                | "feynman"
                | "fmffile"
                | "fmfgraph"
                | "fmfgraph*"
                | "fmfchar"
                | "fmfchar*"
                | "fmfsubgraph"
                | "fmfshrink"
                | "fmffor"
                | "fmfgroup"
                | "experimental"
                | "pspicture"
                | "pspicture*"
                | "psgraph"
                | "psgraph*"
                | "psclip"
                | "psmatrix"
                | "TeXtoEPS"
                | "psfrags"
                | "postscript"
                | "pgfpicture"
                | "picture"
        )
}
