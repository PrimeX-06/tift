use super::{BTreeMap, CommandRule};

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
}
