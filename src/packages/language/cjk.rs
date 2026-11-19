//! Public author interfaces from ctex, xeCJK, xpinyin, zhnumber, zhlipsum,
//! CJK, CJKfntef, CJKnumb, xeCJKfntef and luatexja-fontspec.

use crate::{config::CommandRule, syntax::arguments::signature_end};
use std::collections::BTreeMap;

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    match name {
        "CJK" | "CJK*" => Some("omm"),
        "pinyinscope" => Some("o"),
        "CJKfilltwosides" | "CJKfilltwosides*" => Some("om"),
        _ => None,
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

// Data prefixes include settings, font names, pronunciation data and deferred
// code. The final text arguments of decoration commands remain formatable.
commands! {
    "ctexset" | "zihao" | "ziju" => ("m", "m"),
    "CTEXsetup" => ("om", "om"),
    "CTEXoptions" => ("o", "o"),
    "xeCJKsetup" | "normalspacedchars" | "defaultCJKfontfeatures"
        | "xeCJKsetemboldenfactor" | "xeCJKsetslantfactor" | "punctstyle"
        | "CJKsetecglue" => ("m", "m"),
    "xeCJKDeclareCharClass" | "xeCJKDeclareSubCJKBlock" | "xeCJKsetwidth" => ("s m m", "s m m"),
    "xeCJKCancelSubCJKBlock" | "xeCJKRestoreSubCJKBlock" => ("s m", "s m"),
    "xeCJKsetkern" | "xeCJKsetcharclass" => ("mmm", "mmm"),
    "xeCJKDeclarePunctStyle" | "xeCJKEditPunctStyle" => ("mm", "mm"),
    "setCJKmainfont" | "setCJKromanfont" | "setCJKsansfont" | "setCJKmonofont"
        | "setCJKmathfont" | "CJKfontspec" => ("omo", "omo"),
    "setCJKfamilyfont" | "setCJKfallbackfamilyfont" => ("momo", "momo"),
    "newCJKfontfamily" => ("omomo", "omomo"),
    "addCJKfontfeatures" => ("s o m", "s o m"),
    "CJKfamily" => ("t+ t- m", "t+ t- m"),
    "CJKunderline" | "CJKunderwave" | "CJKunderdblline" | "CJKsout" | "CJKxout"
        | "xeCJKfntefon" => ("s t- s o m", "s t- s o"),
    "CJKunderanyline" => ("s t- s o m m m", "s t- s o m m"),
    "CJKunderanysymbol" => ("ommm", "omm"),
    "CJKunderdot" => ("om", "o"),
    "varCJKunderline" => ("m", ""),
    "xpinyinsetup" => ("m", "m"),
    "xpinyin" => ("omm", "o"),
    "pinyin" => ("om", "om"),
    "setpinyin" => ("mm", "mm"),
    "xpinyinvalue" | "xpinyininitial" | "xpinyinshengmu" | "xpinyinyunmu" => ("s m", "s m"),
    "disablepinyin" => ("s ", "s "),
    "enablepinyin" => ("", ""),
    "zhnumsetup" | "zhrodsetup" | "zhweekday" | "zhtime" => ("m", "m"),
    "zhnumber" | "zhnum" | "zhdig" | "zhnumExtendScaleMap" => ("om", "om"),
    "zhdigits" => ("s o m", "s o m"),
    "zhdate" => ("s m", "s m"),
    "zhnumberwithoptions" | "zhnumwithoptions" | "zhdigwithoptions" => ("mm", "mm"),
    "zhdigitswithoptions" => ("mmm", "mmm"),
    "zhnumClearWrapper" | "zhnumResetWrapper" => ("", ""),
    "zhlipsum" => ("s o o", "s o o"),
    "newzhlipsum" => ("mm", "mm"),
    "CJKchar" => ("omm", "omm"),
    "CJKenc" | "CJKshape" | "CJKcaption" | "CJKnumber" => ("m", "m"),
    "CJKfontenc" | "CJKencfamily" | "CJKencshape" => ("mm", "mm"),
    "CJKdigits" => ("s m", "s m"),
    "CJKspace" | "CJKnospace" | "CJKtilde" | "CJKCJKchar" | "CJKhangulchar"
        | "CJKlatinchar" | "CJKhwkatakana" | "CJKnohwkatakana"
        | "xeCJKResetPunctClass" | "xeCJKResetCharClass" => ("", ""),
    "setmainjfont" | "setsansjfont" | "setmonojfont" | "jfontspec" => ("omo", "omo"),
    "newjfontfamily" | "renewjfontfamily" | "setjfontfamily" | "providejfontfamily"
        | "newjfontface" | "renewjfontface" | "setjfontface" | "providejfontface" => ("momo", "momo"),
    "defaultjfontfeatures" => ("t+ o m", "t+ o m"),
    "addjfontfeatures" | "addjfontfeature" => ("m", "m"),
}

fn invocation_shape(source: &str, name: &str, cursor: usize, shape: &'static str) -> &'static str {
    let base = name.trim_end_matches('*');
    if base == "xpinyin" {
        if shape == "o" {
            return if name.ends_with('*') {
                "o"
            } else if source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'*') {
                "s o"
            } else {
                "omm"
            };
        }
        return if name.ends_with('*') {
            "om"
        } else if source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'*') {
            "s o m"
        } else {
            "omm"
        };
    }
    // xeCJK accepts trailing font options only when leading font options were
    // absent. LuaTeX-ja accepts both sets and retains its independent grammar.
    let prefix = match base {
        "setCJKmainfont" | "setCJKromanfont" | "setCJKsansfont" | "setCJKmonofont"
        | "setCJKmathfont" | "CJKfontspec" => Some(("", "om")),
        "setCJKfamilyfont" | "setCJKfallbackfamilyfont" => Some(("m", "mom")),
        "newCJKfontfamily" => Some(("om", "omom")),
        _ => None,
    };
    if let Some((prefix, without_trailing)) = prefix
        && let Some(end) = signature_end(source, cursor, prefix)
        && source.as_bytes().get(crate::trivia::skip(source, end)) == Some(&b'[')
    {
        return without_trailing;
    }
    shape
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    Some(invocation_shape(
        source,
        name,
        cursor,
        specification(name)?.0,
    ))
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    specification(name).is_some()
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let (shape, data) = specification(name)?;
    let shape = invocation_shape(source, name, cursor, shape);
    let data = invocation_shape(source, name, cursor, data);
    signature_end(
        source,
        crate::syntax::arguments::invocation_cursor(source, cursor, shape),
        shape,
    )?;
    signature_end(
        source,
        crate::syntax::arguments::invocation_cursor(source, cursor, data),
        data,
    )
    .filter(|&end| end > cursor)
}
