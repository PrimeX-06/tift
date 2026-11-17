//! Public argument grammar for page styles, headings, spacing and typography.

use crate::config::CommandRule;
use std::collections::BTreeMap;

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // setspace.sty:489--546. Body text follows the optional spacing factor.
        "spacing" => "m",
        "singlespace" | "singlespace*" | "onehalfspace" | "doublespace" => "",
        _ => return None,
    })
}

pub(crate) fn is_block(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn preserves_tight_whitespace(name: &str) -> bool {
    // onehalfspace/doublespace change line spacing before ending the paragraph.
    matches!(name, "onehalfspace" | "doublespace")
}

macro_rules! commands {
    ($( $($name:literal)|+ => $signature:literal ),+ $(,)?) => {
        fn fixed_signature(name: &str) -> Option<&'static str> {
            Some(match name.trim_end_matches('*') {
                $( $($name)|+ => $signature, )+
                _ => return None,
            })
        }

        pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
            $(
                let arguments = $signature.bytes().filter(|&slot| slot == b'm').count();
                let optional_arguments = $signature.contains('o');
                for name in [$($name),+] {
                    commands.insert(name.into(), CommandRule {
                        arguments: Some(arguments), optional_arguments,
                        ..Default::default()
                    });
                }
            )+
        }
    };
}

commands! {
    // fancyhdr.sty:144--149,193--198,378,410--429,687--691,698--711,769.
    "fancyhead" | "fancyfoot" | "fancyhf" | "fancyheadoffset" | "fancyfootoffset"
        | "fancyhfoffset" => "om",
    "fancyheadwidth" | "fancyfootwidth" | "fancyhfwidth" | "fancyhdrbox" => "oom",
    "fancyheadinit" | "fancyfootinit" | "fancyhfinit" | "fancyfootalign" => "m",
    "fancypagestyle" => "mom",
    "fancypagestyleassign" | "fancyplain" | "iftopfloat" | "ifbotfloat"
        | "iffloatpage" | "iffootnote" => "mm",
    "fancycenter" => "oommm",
    // titlesec.sty:112--153,645--705,1088--1128,1381,1523.
    "titleformat" => "mommmmo",
    "titlespacing" => "mmmmo",
    "titleclass" => "mmo",
    "titleline" => "om",
    "titlerule" => "o",
    "assignpagestyle" => "mm",
    "titlelabel" => "m",
    // setspace.sty:304--360.
    "setstretch" | "SetSinglespace" => "m",
    "singlespacing" | "onehalfspacing" | "doublespacing" => "",
    // microtype.sty:64--90,1449--1457,2117--2124,2339--2346,2465--2473.
    "DeclareMicrotypeSet" | "SetProtrusion" | "SetExpansion" | "SetTracking"
        | "SetExtraKerning" | "SetExtraSpacing" | "DeclareCharacterInheritance" => "omm",
    "UseMicrotypeSet" | "DeclareMicrotypeSetDefault" | "DisableLigatures" | "textls" => "om",
    "DeclareMicrotypeVariants" | "LoadMicrotypeFile" | "DeclareMicrotypeFilePrefix"
        | "microtypesetup" | "microtypecontext" | "leftprotrusion" | "rightprotrusion"
        | "lslig" => "m",
    "DeclareMicrotypeAlias" | "DeclareMicrotypeBabelHook" | "textmicrotypecontext" => "mm",
    "lsstyle" | "noprotrusion" | "noprotrusionifhmode" => "",
    // fontspec-xetex.sty:534--668. Options may follow the font name as well.
    "fontspec" | "setmainfont" | "setsansfont" | "setmonofont" | "setmathrm"
        | "setboldmathrm" | "setmathsf" | "setmathtt" | "setromanfont" => "omo",
    "newfontfamily" | "renewfontfamily" | "setfontfamily" | "providefontfamily"
        | "newfontface" | "renewfontface" | "setfontface" | "providefontface" => "momo",
    "defaultfontfeatures" => "om",
    "addfontfeatures" | "addfontfeature" | "DeclareFontExtensions" | "liningnums" => "m",
    "newfontfeature" | "aliasfontfeature" | "newfontscript" | "newfontlanguage" => "mm",
    "newopentypefeature" | "newICUfeature" | "aliasfontfeatureoption"
        | "IfFontFeatureActiveTF" | "IfFontExistsTF" => "mmm",
    "newAATfeature" => "mmmm",
}

fn next_token(source: &str, cursor: usize) -> Option<u8> {
    source
        .as_bytes()
        .get(crate::trivia::skip(source, cursor))
        .copied()
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    let fixed = fixed_signature(name)?;
    let base = name.trim_end_matches('*');
    let separated_star = !name.ends_with('*') && next_token(source, cursor) == Some(b'*');
    match base {
        "titleformat" if name.ends_with('*') => Some("mm"),
        "titleformat" if separated_star => Some("mmm"),
        "titlerule" if name.ends_with('*') => Some("om"),
        "titlerule" if separated_star => Some("mom"),
        "titlespacing" if separated_star => Some("mmmmmo"),
        "titleline" if separated_star => Some("mom"),
        "fancypagestyle" if separated_star => Some("mmom"),
        "fancyheadwidth" | "fancyfootwidth" | "fancyhfwidth" if separated_star => Some("moom"),
        "textls" if separated_star => Some("mom"),
        "DeclareMicrotypeSet" if separated_star => Some("momm"),
        "DeclareMicrotypeVariants" if separated_star => Some("mm"),
        "defaultfontfeatures" if next_token(source, cursor) == Some(b'+') => Some("mom"),
        "titleclass" => {
            let first = mandatory_end(source, crate::trivia::skip(source, cursor))?;
            Some(if next_token(source, first) == Some(b'[') {
                "mom"
            } else {
                "mmo"
            })
        }
        _ => Some(fixed),
    }
}

fn mandatory_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => crate::syntax::balanced_end(source, start, b'{', b'}'),
        b'\\' => crate::syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' | b']' => None,
        _ => source[start..].chars().next().map(|c| start + c.len_utf8()),
    }
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    fixed_signature(name).is_some()
}

/// Settings and delayed page-style/title code are data. Printing commands keep
/// their final content argument available for ordinary nested formatting.
pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let base = name.trim_end_matches('*');
    let signature = match base {
        "textls" => {
            if !name.ends_with('*') && next_token(source, cursor) == Some(b'*') {
                "mo"
            } else {
                "o"
            }
        }
        "textmicrotypecontext" | "IfFontExistsTF" | "IfFontFeatureActiveTF" => "m",
        "fancyhdrbox" | "fancycenter" => "oo",
        "titleline" => {
            if !name.ends_with('*') && next_token(source, cursor) == Some(b'*') {
                "mo"
            } else {
                "o"
            }
        }
        "leftprotrusion" | "rightprotrusion" | "lslig" | "liningnums" | "iftopfloat"
        | "ifbotfloat" | "iffloatpage" | "iffootnote" | "fancyplain" => return None,
        _ => command_signature(source, name, cursor)?,
    };
    let mut end = cursor;
    for slot in signature.bytes() {
        let start = crate::trivia::skip(source, end);
        match slot {
            b'o' if source.as_bytes().get(start) == Some(&b'[') => {
                end = crate::syntax::balanced_end(source, start, b'[', b']')?;
            }
            b'o' => (),
            b'm' => end = mandatory_end(source, start)?,
            _ => return None,
        }
    }
    (end > cursor).then_some(end)
}
