//! Literal package grammar for equation wrappers, measured floats and notes.

use crate::config::CommandRule;
use std::collections::BTreeMap;

// empheq.sty:832; cases.sty:47,124. The display wrapper's options and
// left-hand expression precede the alignment body.
pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &["empheq", "numcases", "subnumcases"];

// threeparttable.sty:258--274 and threeparttablex.sty:36--40,60--63.
// TableNotes stores a tablenotes body for insertion into a longtable.
pub(crate) const LIST_ENVIRONMENTS: &[&str] = &["tablenotes", "TableNotes"];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "empheq" => "om",
        "numcases" | "subnumcases" => "m",
        // threeparttable.sty:107,122 uses optional vertical placement.
        "threeparttable" | "measuredfigure" | "tablenotes" | "TableNotes" => "o",
        // threeparttablex.sty:70 is a scope wrapper for longtable notes.
        "ThreePartTable" => "",
        // cuted.sty:229--270 places a full-width vbox in two-column output.
        "strip" => "",
        // thm-restate.sty:103,186--194: [title]{theorem environment}{macro name}.
        "restatable" | "restatable*" => "omm",
        _ => return None,
    })
}

pub(crate) fn command_signature(_source: &str, name: &str, _cursor: usize) -> Option<&'static str> {
    Some(match name.trim_end_matches('*') {
        // float.sty and newfloat.sty keep the extension/placement metadata
        // separate from the printable caption and body.
        "newfloat" => "mmmo",
        "DeclareFloatingEnvironment" => "om",
        "SetupFloatingEnvironment" | "floatname" => "mm",
        "floatstyle" => "m",
        "restylefloat" => "om",
        // subfig.sty: [list entry][caption]{subfloat body}.
        "subfloat" => "oom",
        "subref" => "m",
        // bookcover.cls: component type, part, optional four margins, content.
        "bookcovercomponent" => "mmom",
        _ => return None,
    })
}

pub(crate) fn is_box(name: &str) -> bool {
    matches!(name, "threeparttable" | "measuredfigure")
}

pub(crate) fn is_block(name: &str) -> bool {
    header_signature(name).is_some() && !is_box(name)
}

/// Fixed command arity bounds argument ownership through the normal parser.
/// Source declarations and explicit command rules retain their usual priority.
pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // accents.sty:101,179,237--239, including the one-argument palette.
            ("accentset", 2, false),
            ("underaccent", 2, false),
            ("undertilde", 1, false),
            // empheq.sty:820,839,853.
            ("empheqset", 1, false),
            ("DeclareLeftDelimiter", 1, true),
            ("DeclareRightDelimiter", 1, true),
            // threeparttable.sty:288; threeparttablex.sty:29,43,80--82.
            ("tnote", 1, false),
            ("tnotex", 1, false),
            ("setTableNoteFont", 1, false),
            ("insertTableNotes", 0, false),
            // thm-kv.sty:333,522 and thm-listof.sty:45.
            ("declaretheorem", 1, true),
            ("declaretheoremstyle", 1, true),
            ("listoftheorems", 0, true),
            // thm-patch.sty:164--176 appends a mandatory hook after the option.
            ("addtotheorempreheadhook", 1, true),
            ("addtotheorempostheadhook", 1, true),
            ("addtotheoremprefoothook", 1, true),
            ("addtotheorempostfoothook", 1, true),
            // needspace.sty:7,24--45. Needspace accepts a starred form.
            ("needspace", 1, false),
            ("Needspace", 1, false),
        ],
    );
}
