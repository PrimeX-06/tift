//! Public argument grammar for typography, positioned text and box packages.

use crate::{config::CommandRule, syntax::arguments::signature_end};
use std::collections::BTreeMap;

pub(crate) const LIST_ENVIRONMENTS: &[&str] = &[
    "epigraphs",
    "dinglist",
    "dingautolist",
    "Pilist",
    "Piautolist",
];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "epigraphs" | "flushepinormal" | "flushleftright" | "Sbox" => "",
        // textpos: {width}[reference position](coordinates).
        "textblock" | "textblock*" => "mop",
        "mcellbox" | "Bcenter" | "Bflushleft" | "Bflushright" => "o",
        "dinglist" | "dingautolist" => "m",
        "Pilist" | "Piautolist" => "mm",
        "Blist" => "mmo",
        "Landscape" | "Landscape*" => "mmm",
        "LandScape" => "m",
        "landfloat" => "mm",
        _ => return None,
    })
}

pub(crate) fn environment_for_command(name: &str) -> Option<&str> {
    if header_signature(name).is_some() {
        Some(name)
    } else {
        name.strip_prefix("end")
            .filter(|name| header_signature(name).is_some())
    }
}

pub(crate) fn is_block(name: &str) -> bool {
    header_signature(name).is_some() && !matches!(name, "Sbox" | "mcellbox")
}

macro_rules! commands {
    ($( $($name:literal)|+ => ($shape:literal, $data:literal) ),+ $(,)?) => {
        fn specification(name: &str) -> Option<(&'static str, &'static str)> {
            Some(match name.trim_end_matches('*') {
                $( $($name)|+ => ($shape, $data), )+
                _ => return None,
            })
        }

        pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
            $(
                for name in [$($name),+] {
                    commands.insert(name.into(), CommandRule {
                        arguments: Some($shape.bytes().filter(|&slot| slot == b'm').count()),
                        optional_arguments: $shape.contains('o'),
                        ..Default::default()
                    });
                }
            )+
        }
    };
}

// The second shape protects only leading settings or delayed code. Printable
// content remains available for nested formatting.
commands! {
    "afterpage" => ("m", "m"),
    "epigraph" | "qitem" => ("mm", ""),
    "epigraphhead" => ("om", "o"),
    "qitemlabel" | "dropchapter" => ("m", "m"),
    "undodrop" | "epigraphnoindent" => ("", ""),
    "lettrine" => ("omm", "o"),
    "LettrineOptionsFor" => ("mm", "mm"),
    "relsize" | "relscale" => ("m", "m"),
    "larger" | "smaller" => ("o", "o"),
    "textlarger" | "textsmaller" => ("om", "o"),
    "textscale" => ("mm", "m"),
    "mathlarger" | "mathsmaller" => ("m", ""),
    "TPGrid" => ("omm", "omm"),
    "TPMargin" | "TPReferencePosition" | "TPoptions" | "textblocklabel"
        | "textblockcolour" | "textblockcolor" | "textblockrulecolour"
        | "textblockrulecolor" => ("m", "m"),
    "TPShowGrid" | "textblockorigin" => ("mm", "mm"),
    "makecell" | "thead" | "rotcell" | "rothead" | "makecellbox" | "gape" => ("om", "o"),
    "multirowcell" | "multirowthead" => ("moom", "moo"),
    "Gape" => ("oom", "oo"),
    "setcellgapes" => ("om", "om"),
    "makegapedcells" | "nomakegapedcells" => ("", ""),
    "negjot" => ("m", ""),
    "diaghead" => ("pmmm", "p"),
    "Xhline" | "eline" => ("m", "m"),
    "Xcline" => ("mm", "mm"),
    "erows" => ("omm", "omm"),
    "Xrows" => ("mm", "mm"),
    "ding" | "dingfill" | "dingline" | "Pifont" => ("m", "m"),
    "Pisymbol" | "Pifill" | "Piline" | "Pinumber" => ("mm", "mm"),
    "cancel" | "bcancel" | "xcancel" => ("m", ""),
    "cancelto" => ("mm", ""),
    "diagbox" => ("omm", "o"),
    "slashbox" | "backslashbox" => ("oomm", "oo"),
    "shadowbox" | "doublebox" | "ovalbox" | "Ovalbox" => ("m", ""),
    "cornersize" => ("m", "m"),
    "fancyoval" => ("po", "po"),
    "boxput" => ("pmm", "p"),
    "fancypage" | "thisfancypage" => ("mm", "mm"),
    "TheSbox" | "UsePageParameters" => ("", ""),
    "marginnote" => ("omo", ""),
    // Option domains retain settings without hiding the surrounding math atom.
    "nicefrac" => ("omm", ""),
    "sfrac" => ("omom", ""),
    "uline" | "uuline" | "uwave" | "sout" | "xout" | "dashuline" | "dotuline" => ("m", ""),
    "markoverwith" => ("m", "m"),
    "useunder" => ("mmm", "mmm"),
    "normalem" | "ULforem" => ("", ""),
    "newfootnote" | "SelectFootnoteRule" => ("om", "om"),
    "DeclareNewFootnote" => ("omo", "omo"),
    "SetFootnoteHook" | "ExtraParaSkip" => ("m", "m"),
    "SplitNote" => ("", ""),
}

fn separated_star(source: &str, name: &str, cursor: usize) -> bool {
    !name.ends_with('*')
        && source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'*')
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    let (shape, _) = specification(name)?;
    let base = name.trim_end_matches('*');
    if separated_star(source, name, cursor) {
        match base {
            "makecell" | "thead" => return Some("mom"),
            "TPMargin" | "cornersize" => return Some("mm"),
            "TPShowGrid" => return Some("mmm"),
            "boxput" => return Some("mpmm"),
            _ => (),
        }
    }
    if base == "diagbox" {
        let end = signature_end(source, cursor, shape)?;
        if source.as_bytes().get(crate::trivia::skip(source, end)) == Some(&b'{') {
            return Some("ommm");
        }
    }
    Some(shape)
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    specification(name).is_some()
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let (_, data) = specification(name)?;
    let data = if separated_star(source, name, cursor) {
        match name {
            "makecell" | "thead" => "mo",
            "TPMargin" | "cornersize" => "mm",
            "TPShowGrid" => "mmm",
            "boxput" => "mp",
            _ => data,
        }
    } else {
        data
    };
    signature_end(source, cursor, data).filter(|&end| end > cursor)
}
