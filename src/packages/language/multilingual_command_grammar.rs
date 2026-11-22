//! Language selection and stored translation declarations.
//!
//! Identifiers and delayed replacement text remain data. Directly printed
//! foreign-language text stays available to ordinary source formatting.

use crate::{config::CommandRule, syntax::arguments::signature_end};
use std::{collections::BTreeMap, ops::Range};

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "otherlanguage" | "otherlanguage*" => "om",
        _ => return None,
    })
}

crate::packages::argument_tables::argument_commands! {
    // babel.sty:641,819,1044,1702,2224. Language names, tags and patterns
    // are identifiers; foreign-language text continues through formatting.
    "selectlanguage" | "babelprovide" | "babelhyphenation" => ("om", "om"),
    "foreignlanguage" | "textlang" => ("omm", "om"),
    "babeltags" => ("m", "m"),
    "babelfont" => ("omomo", "omomo"),
    "shorthandon" | "shorthandoff" | "useshorthands" => ("m", "m"),
    "defineshorthand" => ("omm", "omm"),
    "languageshorthands" => ("m", "m"),
    "languageattribute" => ("mm", "mm"),
    // polyglossia.sty:1021,1174,1224,1241,1279,1412.
    "setmainlanguage" | "setdefaultlanguage" | "resetdefaultlanguage"
        | "setotherlanguage" => ("om", "om"),
    "setotherlanguages" => ("m", "m"),
    "SetGlossOptions" => ("mm", "mm"),
    "setlanguagealias" => ("s O{} m m", "s O{} m m"),
    // translations.sty:142,171,230,333--372,445--470. Translation
    // replacements are stored now and can contain unbalanced TeX fragments.
    "DeclareLanguage" | "baselanguage" => ("m", "m"),
    "DeclareLanguageDialect" | "DeclareLanguageAlias" => ("mm", "mm"),
    "DeclareTranslation" | "NewTranslation" | "RenewTranslation" | "ProvideTranslation"
        | "declaretranslation" | "definetranslation" | "redefinetranslation"
        | "addtranslation" => ("mmm", "mmm"),
    "DeclareTranslationFallback" | "NewTranslationFallback" | "RenewTranslationFallback"
        | "ProvideTranslationFallback" | "declaretranslationfallback"
        | "definetranslationfallback" | "redefinetranslationfallback"
        | "addtranslationfallback" => ("mm", "mm"),
    "GetTranslation" | "GetLCTranslation" => ("m", "m"),
    "GetTranslationFor" | "GetLCTranslationFor" => ("mm", "mm"),
    "IfTranslation" => ("mmmm", "mm"),
    "ifcurrentlanguage" | "ifcurrentbaselanguage" | "ifcurrentlang"
        | "ifcurrentbaselang" => ("mmm", "m"),

}

pub(crate) fn environment_for_command(name: &str) -> Option<&str> {
    if header_signature(name).is_some() {
        Some(name)
    } else {
        name.strip_prefix("end")
            .filter(|name| header_signature(name).is_some())
    }
}

fn starred(source: &str, name: &str, cursor: usize) -> bool {
    name.ends_with('*') || source.as_bytes().get(crate::trivia::skip(source, cursor)) == Some(&b'*')
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    if name.trim_end_matches('*') == "foreignlanguage" && starred(source, name, cursor) {
        return Some(if name.ends_with('*') { "mm" } else { "mmm" });
    }
    specification(name).map(|(shape, _)| shape)
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    specification(name).is_some_and(|(_, prefix)| !prefix.is_empty())
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let (_, mut prefix) = specification(name)?;
    if prefix.is_empty() {
        return None;
    }
    if name.trim_end_matches('*') == "foreignlanguage" && starred(source, name, cursor) {
        prefix = if name.ends_with('*') { "m" } else { "mm" };
    }
    signature_end(source, cursor, command_signature(source, name, cursor)?)?;
    signature_end(source, cursor, prefix).filter(|finish| *finish > cursor)
}

pub(crate) fn command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    command_signature(source, name, cursor)?;
    Some(
        command_header_end(source, name, cursor)
            .map(|end| std::iter::once(cursor..end).collect())
            .unwrap_or_default(),
    )
}

pub(crate) fn command_branch_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    let prefix = match name {
        "IfTranslation" => "mm",
        "ifcurrentlanguage" | "ifcurrentbaselanguage" | "ifcurrentlang" | "ifcurrentbaselang" => {
            "m"
        }
        _ => return None,
    };
    let mut cursor = signature_end(source, cursor, prefix)?;
    let mut result = Vec::new();
    for _ in 0..2 {
        let start = crate::trivia::skip(source, cursor);
        cursor = signature_end(source, cursor, "m")?;
        result.push(if source.as_bytes()[start] == b'{' {
            start + 1..cursor - 1
        } else {
            start..cursor
        });
    }
    Some(result)
}
