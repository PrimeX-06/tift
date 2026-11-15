//! Literal interfaces of TikZ plot, circuit, timeline and network packages.
//!
//! Inline data readers preserve their physical lines and spacing. Source
//! declarations and explicit syntax are resolved before these defaults.

use crate::config::CommandRule;
use crate::syntax;
use crate::trivia::skip;
use std::collections::BTreeMap;
use std::ops::Range;

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &["quantikz", "tikztimingtable"];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // pgfplots.code.tex:12702--12733; groupplots:460--478. The axis
        // factory used by polar/smithchart retains the same leading option.
        "axis" | "semilogxaxis" | "semilogyaxis" | "loglogaxis" | "polaraxis" | "smithchart"
        | "groupplot" => "o",
        // quantikz2:51 O{}+b; tikz-timing.sty:1093 has two delimited cells.
        "quantikz" | "tikztimingtable" => "o",
        // tikz-timing.sty:1126--1155,1184: extra drawing code between rows.
        "extracode" => "o",
        // pgfgantt.sty:312 and tikz-network.sty:1127.
        "ganttchart" => "omm",
        "Layer" => "o",
        _ => return None,
    })
}

/// Environments whose bodies expose the literal TikZ drawing grammar.
pub(crate) fn is_tikz_environment(name: &str) -> bool {
    matches!(
        name,
        "axis"
            | "semilogxaxis"
            | "semilogyaxis"
            | "loglogaxis"
            | "polaraxis"
            | "smithchart"
            | "groupplot"
            | "ganttchart"
            | "Layer"
            | "extracode"
    )
}

pub(crate) fn is_box(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn is_non_prose(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn preserves_tight_whitespace(name: &str) -> bool {
    header_signature(name).is_some()
}

/// Ordered optional arguments delimit gate sizes and network coordinates.
pub(crate) fn command_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // tikzlibraryquantikz2.code.tex:184,233,801,829,1105,1197.
        "gate" | "meter" | "metercw" | "wire" => "ooom",
        "ghost" | "wireoverride" => "oom",
        // tikz-network.sty:585--586. Coordinates are separate parentheses.
        "Edge" => "opp",
        _ => return None,
    })
}

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // pgfplots legend/options interfaces; groupplots:137--140.
            ("pgfplotsset", 1, false),
            ("usepgfplotslibrary", 1, false),
            ("nextgroupplot", 0, true),
            ("addlegendentry", 1, true),
            ("addlegendimage", 1, true),
            ("addlegendentryexpanded", 1, true),
            ("legend", 1, false),
            ("pgfplotsinvokeforeach", 2, false),
            // Its destination also has a delimited `to` form; ownership here
            // stops after the first data argument common to all forms.
            ("pgfplotstableread", 1, true),
            // quantikz2 public circuit commands. The exact signatures above
            // bound commands that accept two/three leading bracket options.
            ("gate", 1, true),
            ("ghost", 1, true),
            ("meter", 1, true),
            ("metercw", 1, true),
            ("wire", 1, true),
            ("wireoverride", 1, true),
            ("ctrl", 1, true),
            ("octrl", 1, true),
            ("control", 1, true),
            ("ocontrol", 1, true),
            ("targ", 1, true),
            ("targX", 1, true),
            ("swap", 1, true),
            ("phase", 1, true),
            ("measure", 1, true),
            ("measuretab", 1, true),
            ("meterD", 1, true),
            ("inputD", 1, true),
            ("trash", 1, true),
            ("ground", 1, true),
            ("makeebit", 1, true),
            ("qwbundle", 1, true),
            ("setwiretype", 1, true),
            ("lstick", 1, true),
            ("rstick", 1, true),
            ("midstick", 1, true),
            ("slice", 1, true),
            ("gateinput", 1, true),
            ("gateoutput", 1, true),
            ("gategroup", 1, true),
            ("wave", 0, true),
            ("vcw", 1, false),
            ("vqw", 1, false),
            ("qw", 0, false),
            ("cw", 0, false),
            ("expandedwire", 4, false),
            ("permute", 1, false),
            ("phantomgate", 1, false),
            ("hphantomgate", 1, false),
            // pgfgantt.sty:428,460,531,570,996--1025,1609. Starred
            // milestone factories produce two mandatory argument commands.
            ("ganttnewline", 0, true),
            ("ganttvrule", 2, true),
            ("gantttitle", 2, true),
            ("gantttitlelist", 2, true),
            ("ganttbar", 3, true),
            ("ganttlinkedbar", 3, true),
            ("ganttgroup", 3, true),
            ("ganttlinkedgroup", 3, true),
            ("ganttmilestone", 2, true),
            ("ganttlinkedmilestone", 2, true),
            ("ganttlink", 2, true),
            ("setganttlinklabel", 2, false),
            // tikz-network.sty:257,277,295,312,338,365--369,406,771,924.
            ("Vertex", 1, true),
            ("Edge", 0, true),
            ("Vertices", 1, true),
            ("Edges", 1, true),
            ("SetVertexStyle", 0, true),
            ("SetEdgeStyle", 0, true),
            ("SetPlaneStyle", 0, true),
            ("SetTextStyle", 0, true),
            ("SetCoordinates", 0, true),
            ("SetLayerDistance", 1, false),
            ("SetDefaultUnit", 1, false),
            ("SetDistanceScale", 1, false),
            ("SetPlaneWidth", 1, false),
            ("SetPlaneHeight", 1, false),
            ("Plane", 0, true),
            ("Text", 1, true),
            // tikz-3dplot.sty:59--107,245--278,350--412,435,457,531,618.
            ("tdplotsinandcos", 3, false),
            ("tdplotmult", 3, false),
            ("tdplotdiv", 3, false),
            ("tdplotcheckdiff", 5, false),
            ("tdplotsetmaincoords", 2, false),
            ("tdplottransformmainrot", 3, false),
            ("tdplottransformrotmain", 3, false),
            ("tdplottransformmainscreen", 3, false),
            ("tdplotsetrotatedcoords", 3, false),
            ("tdplotsetrotatedcoordsorigin", 1, false),
            ("tdplotsetthetaplanecoords", 1, false),
            ("tdplotsetrotatedthetaplanecoords", 1, false),
            ("tdplotsetcoord", 4, false),
            ("tdplotsimplesetcoord", 4, false),
            ("tdplotdrawarc", 6, true),
            ("tdplotdrawpolytopearc", 3, true),
            ("tdplotsphericalsurfaceplot", 8, true),
            ("tdplotsetpolarplotrange", 4, false),
            ("tdplotdosurfaceplot", 6, false),
            ("tdplotshowargcolorguide", 4, false),
            ("tdplotgetpolarcoords", 3, false),
            // tikz-timing.sty:265,2321.
            ("texttiming", 1, true),
            ("usetikztiminglibrary", 1, true),
        ],
    );
}

fn option_end(source: &str, cursor: usize) -> Option<usize> {
    let cursor = skip(source, cursor);
    if source.as_bytes().get(cursor) == Some(&b'[') {
        syntax::balanced_end(source, cursor, b'[', b']')
    } else {
        Some(cursor)
    }
}

fn keyword_end(source: &str, cursor: usize, keyword: &str) -> Option<usize> {
    let end = cursor.checked_add(keyword.len())?;
    (source.get(cursor..end)? == keyword
        && source
            .as_bytes()
            .get(end)
            .is_none_or(|next| !next.is_ascii_alphabetic()))
    .then_some(end)
}

/// Physical-line table records and compact timing DSL belong to their reader.
/// Only literal, balanced invocations are recognized. The caller checks source
/// bindings and configuration before applying package grammar.
pub(crate) fn literal_data_range(
    source: &str,
    command_end: usize,
    name: &str,
) -> Option<Range<usize>> {
    if !matches!(name, "texttiming" | "pgfplotstableread" | "addplot") {
        return None;
    }
    let mut cursor = skip(source, command_end);
    match name {
        "texttiming" | "pgfplotstableread" => cursor = option_end(source, cursor)?,
        "addplot" => {
            // The digit is a token suffix, since TeX control words use letters.
            if source.as_bytes().get(cursor) == Some(&b'3') {
                cursor = skip(source, cursor + 1);
            }
            if source.as_bytes().get(cursor) == Some(&b'+') {
                cursor = skip(source, cursor + 1);
            }
            cursor = skip(source, option_end(source, cursor)?);
            if let Some(end) = keyword_end(source, cursor, "plot") {
                cursor = skip(source, option_end(source, end)?);
            }
            cursor = keyword_end(source, cursor, "table")?;
            cursor = option_end(source, cursor)?;
        }
        _ => return None,
    }
    cursor = skip(source, cursor);
    Some(cursor..syntax::balanced_end(source, cursor, b'{', b'}')?)
}


