use super::{BTreeMap, CommandRule};

pub(crate) fn explicitly_disables_todonotes(options: &str) -> bool {
    let mut disabled = false;
    for option in options.split(',').map(str::trim_ascii) {
        match option {
            "disable" => disabled = true,
            "catalan" | "croatian" | "danish" | "dutch" | "english" | "french" | "german"
            | "italian" | "ngerman" | "portuguese" | "spanish" | "swedish" | "obeyDraft"
            | "draft" | "draftcls" | "draftclsnofoot" | "obeyFinal" | "final"
            | "colorinlistoftodos" | "dvistyle" | "prependcaption" | "shadow"
            | "loadshadowlibrary" => (),
            _ => return false,
        }
    }
    disabled
}

pub(crate) fn disabled_missingfigure_end(source: &str, end: usize) -> Option<usize> {
    let argument_start = |cursor| {
        let start = crate::trivia::skip(source, cursor);
        memchr::memchr_iter(b'\n', &source.as_bytes()[cursor..start])
            .nth(1)
            .is_none()
            .then_some(start)
    };
    let mut start = argument_start(end)?;
    if source.as_bytes().get(start) == Some(&b'[') {
        let option_end = crate::syntax::arguments::signature_end(source, start + 1, "!u{]}")?;
        start = argument_start(option_end)?;
    }
    if source.as_bytes().get(start) == Some(&b'\\') {
        let (control, _) = crate::syntax::command_at(source, start)?;
        if !control
            .trim_end_matches('*')
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic())
        {
            return None;
        }
    } else if source[start..].starts_with("^^") {
        return None;
    }
    crate::syntax::arguments::signature_end(source, start, "m")
}

pub(crate) fn missingfigure_binding_uncertain(source: &str, name: &str, end: usize) -> bool {
    let base = name.trim_end_matches('*');
    if matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet")
        && crate::syntax::command_at(source, crate::trivia::skip(source, end))
            .is_some_and(|(target, _)| target == "csname")
    {
        return true;
    }
    let patch = matches!(
        base,
        "patchcmd" | "apptocmd" | "pretocmd" | "xpatchcmd" | "xapptocmd" | "xpretocmd"
    );
    let finish = match base {
        "@namedef" | "csdef" | "csgdef" | "csedef" | "csxdef" => return true,
        "AtBeginDocument"
        | "AtEndDocument"
        | "AtEndPreamble"
        | "AfterEndPreamble"
        | "AddToHook"
        | "AddToHookNext"
        | "AddToHookWithArguments"
        | "BeforeBeginEnvironment"
        | "AfterEndEnvironment" => crate::packages::commands::command_header_end(source, name, end),
        _ if patch => crate::packages::commands::command_header_end(source, name, end),
        "def"
        | "gdef"
        | "edef"
        | "xdef"
        | "NewDocumentCommand"
        | "RenewDocumentCommand"
        | "DeclareDocumentCommand"
        | "ProvideDocumentCommand" => crate::syntax::delayed_data_end(source, name, end),
        _ if crate::syntax::classic_command_definition(base) => {
            crate::syntax::delayed_data_end(source, name, end)
        }
        _ => return false,
    };
    finish.is_none_or(|finish| {
        let code = &source[end..finish];
        code.contains("\\csname")
            || code.contains("missingfigure")
                && (patch || crate::syntax::has_analysis_declaration_tokens(code))
    })
}

pub(super) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "anfxnote" | "anfxwarning" | "anfxerror" | "anfxfatal" | "afixme" => "om",
        "anfxnote*" | "anfxwarning*" | "anfxerror*" | "anfxfatal*" => "omm",
        _ => return None,
    })
}

commands! {
    // todonotes.sty:34,319--335,534,554,561,581. Note text is printable.
    "todo" | "missingfigure" => ("om", "o"),
    "listoftodos" => ("o", ""),
    "todototoc" => ("", ""),
    "setuptodonotes" => ("m", "m"),
    "todostyle" => ("mm", "mm"),
    // changes.sty:441,476--587,830 and generated ch-prefixed commands.
    "added" | "deleted" | "highlight" | "comment" | "chadded" | "chdeleted"
        | "chhighlight" | "chcomment" => ("om", "o"),
    "replaced" | "chreplaced" => ("omm", "o"),
    "definechangesauthor" => ("om", "om"),
    "listofchanges" => ("o", "o"),
    "setaddedmarkup" | "setdeletedmarkup" | "sethighlightmarkup" | "setcommentmarkup"
        | "setauthormarkup" | "setauthormarkupposition" | "setauthormarkuptext" => ("m", "m"),
    // fixme.sty:481--499,548--579,690,733.
    "fxnote" | "fxwarning" | "fxerror" | "fxfatal" | "fixme" => ("om", "o"),
    "fxsetup" | "fxusetheme" | "fxloadlayouts" | "fxloadenvlayouts"
        | "fxuselayouts" | "fxuseenvlayout" | "fxusetargetlayout" => ("m", "m"),
    "FXRegisterAuthor" => ("mmm", "mmm"),
    "listoffixmes" => ("", ""),
    // pdfcomment.sty: annotation options precede their printable text. Markup
    // and tooltip forms keep both display text arguments formatable.
    "pdfcomment" | "pdfmargincomment" | "pdffreetextcomment" | "pdfsquarecomment"
        | "pdfcirclecomment" | "pdflinecomment" => ("om", "o"),
    "pdfmarkupcomment" | "pdftooltip" => ("omm", "o"),
    "pdfcommentsetup" => ("om", "om"),
    "listofpdfcomments" => ("o", "o"),
    "setliststyle" | "defineliststyle" | "defineavatar" | "definestyle" => ("mm", "mm"),
}
