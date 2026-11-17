//! Literal public grammar from paracol, hyperref, cleveref, xcolor and geometry.
//!
//! Interleaved options and conditional arity are resolved at each invocation.
//! The caller applies explicit configuration and source declarations first.

use crate::config::CommandRule;
use std::collections::BTreeMap;
use std::ops::Range;

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        // paracol.sty:1989--1995,2089--2091: [left columns]*{total}[span].
        "paracol" => "omo",
        // paracol.sty:2462--2467: starred column wrappers read spanning text.
        "nthcolumn" => "m",
        "nthcolumn*" => "mo",
        "leftcolumn" | "rightcolumn" => "",
        "leftcolumn*" | "rightcolumn*" => "o",
        _ => return None,
    })
}

pub(crate) fn is_block(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn invocation_signature(
    source: &str,
    cursor: usize,
    name: &str,
) -> Option<&'static str> {
    if name != "paracol" {
        return None;
    }
    let start = crate::trivia::skip(source, cursor);
    if source.as_bytes().get(start) == Some(&b'[') {
        let end = crate::syntax::balanced_end(source, start, b'[', b']')?;
        let star = crate::trivia::skip(source, end);
        if source.as_bytes().get(star) == Some(&b'*') {
            // The middle mandatory token consumes the confirmed literal star.
            return Some("ommo");
        }
    }
    Some("omo")
}

macro_rules! command_signatures {
    ($( $($name:literal)|+ => $signature:literal ),+ $(,)?) => {
        fn fixed_command_signature(name: &str) -> Option<&'static str> {
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
                        arguments: Some(arguments),
                        optional_arguments,
                        ..Default::default()
                    });
                }
            )+
        }
    };
}

command_signatures! {
    // paracol.sty:2127--2138,2484--2487,2520--2528,2660--2668.
    "columnratio" | "setcolumnwidth" | "marginparthreshold" => "mo",
    "definecolumnpreamble" => "mm",
    "ensurevspace" | "synccounter" | "localcounter" | "globalcounter"
        | "footnoteplacement" | "footnotelayout" => "m",
    "syncallcounters" | "flushpage" | "resetbackgroundcolor" => "",
    "twosided" | "normalcolumncolor" | "normalcolseprulecolor" | "switchcolumn" => "o",
    // paracol.sty:2699--2710,2805--2817: [model]{color}[column].
    "columncolor" | "colseprulecolor" => "omo",
    // paracol.sty:2827--2884: {region}[column][model]{color}.
    "backgroundcolor" => "moom",
    "nobackgroundcolor" => "mo",
    // hyperref.sty:4670,4716,4808--4868; hpdftex.def:1729--1744.
    "href" => "omm",
    "hyperimage" | "hyperlink" | "hypertarget" | "texorpdfstring"
        | "currentpdfbookmark" | "subpdfbookmark" | "belowpdfbookmark" => "mm",
    "hypersetup" | "hyperbaseurl" | "hyperlinkfileprefix"
        | "pdfstringdefDisableCommands" => "m",
    "pdfbookmark" => "omm",
    "hyperref" => "mmmm",
    "hyperdef" => "ommm",
    // cleveref.sty:1146--1186,1264--1277,1851--1877,3544--3610.
    "cref" | "Cref" | "labelcref" | "namecref" | "nameCref" | "lcnamecref"
        | "namecrefs" | "nameCrefs" | "lcnamecrefs" | "cpageref" | "Cpageref"
        | "labelcpageref" | "crefdefaultlabelformat" => "m",
    "crefrange" | "Crefrange" | "cpagerefrange" | "Cpagerefrange" | "crefalias"
        | "creflabelformat" | "crefrangelabelformat" | "crefformat" | "Crefformat"
        | "crefrangeformat" | "Crefrangeformat" | "labelcrefformat"
        | "labelcrefrangeformat" => "mm",
    "crefname" | "Crefname" => "mmm",
    "crefmultiformat" | "Crefmultiformat" | "crefrangemultiformat"
        | "Crefrangemultiformat" | "labelcrefmultiformat"
        | "labelcrefrangemultiformat" => "mmmmm",
    // xcolor.sty:519--522,594--607,637--705,748,820--830,1031--1033,1115.
    "definecolor" | "xdefinecolor" | "providecolor" | "preparecolor" => "omomm",
    "colorlet" => "omom",
    "DefineNamedColor" | "convertcolorspec" => "mmmm",
    "definecolors" | "providecolors" | "blendcolors" => "m",
    "definecolorset" | "providecolorset" | "preparecolorset" => "ommmm",
    "definecolorseries" => "mmmomom",
    "resetcolorseries" | "maskcolors" => "om",
    "colorbox" => "omm",
    "fcolorbox" => "omomm",
    "nopagecolor" => "",
    "extractcolorspec" => "mm",
    "extractcolorspecs" => "mmm",
    // geometry.sty:1169--1208. restoregeometry consumes no argument.
    "geometry" | "newgeometry" | "savegeometry" | "loadgeometry" => "m",
    "restoregeometry" => "",
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    let base = name.trim_end_matches('*');
    match base {
        // hyperref.sty:4816--4832: [label]{text} or {URL}{category}{name}{text}.
        "hyperref" => Some(
            if source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'[') {
                "om"
            } else {
                "mmmm"
            },
        ),
        // paracol.sty:2285--2294: the star selects all counters, without a name.
        "globalcounter" if name.ends_with('*') => Some(""),
        "globalcounter"
            if source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'*') =>
        {
            Some("m")
        }
        // paracol.sty:2388--2409: [column]*[spanning text]. A star immediately
        // after the control word is already included in the command token.
        "switchcolumn" if name.ends_with('*') => Some("o"),
        "switchcolumn" => {
            let start = crate::trivia::skip(source, cursor);
            let after_column = if source.as_bytes().get(start) == Some(&b'[') {
                crate::syntax::balanced_end(source, start, b'[', b']')?
            } else {
                cursor
            };
            Some(
                if source
                    .as_bytes()
                    .get(crate::trivia::skip(source, after_column))
                    == Some(&b'*')
                {
                    "omo"
                } else {
                    "o"
                },
            )
        }
        _ => fixed_command_signature(name),
    }
}

/// hyperref's hyper@normalise changes percent/hash/underscore before scanning
/// the URL. Its printed text argument keeps ordinary TeX tokenization.
pub(crate) fn context_invocation_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    _context: &crate::packages::InvocationContext<'_>,
) -> Option<Result<Vec<crate::syntax::xparse::ArgumentSlot>, Range<usize>>> {
    href_invocation_ranges(source, name, cursor)
}

fn href_invocation_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Result<Vec<crate::syntax::xparse::ArgumentSlot>, Range<usize>>> {
    if name.trim_end_matches('*') != "href" {
        return None;
    }
    let mut cursor = crate::trivia::skip(source, cursor);
    let mut ranges = Vec::new();
    if source.as_bytes().get(cursor) == Some(&b'[') {
        let Some(end) = crate::syntax::balanced_end(source, cursor, b'[', b']') else {
            return Some(Err(cursor..source.len()));
        };
        ranges.push(crate::syntax::xparse::ArgumentSlot {
            range: cursor..end,
            body: cursor + 1..end - 1,
            slot: 0,
        });
        cursor = crate::trivia::skip(source, end);
    }
    if source.as_bytes().get(cursor) != Some(&b'{') {
        return None;
    }
    let Some(end) = crate::syntax::literal_group_end(source, cursor) else {
        return Some(Err(cursor..source.len()));
    };
    ranges.push(crate::syntax::xparse::ArgumentSlot {
        range: cursor..end,
        body: cursor + 1..end - 1,
        slot: 1,
    });
    let text = crate::syntax::xparse::Signature::parse("m")?.slot_ranges(source, end..source.len());
    let Some(text) = text else {
        let start = crate::trivia::skip(source, end);
        return (source.as_bytes().get(start) == Some(&b'{')).then_some(Err(start..source.len()));
    };
    ranges.extend(text.into_iter().map(|mut argument| {
        argument.slot = 2;
        argument
    }));
    Some(Ok(ranges))
}

pub(crate) fn command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    href_invocation_ranges(source, name, cursor)?
        .ok()
        .map(|arguments| {
            arguments
                .into_iter()
                .filter(|argument| argument.slot < 2)
                .map(|argument| argument.range)
                .collect()
        })
}
