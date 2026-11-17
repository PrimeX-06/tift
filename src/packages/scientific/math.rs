//! Positional grammar for common mathematical packages.
//! Shapes are shared by structural ownership and math-domain inference.

#[derive(Clone, Copy)]
pub(crate) struct Signature {
    pub shape: &'static str,
    // M = math; T = text; I = inherited text/math; U = literal data.
    pub domains: &'static str,
}

pub(crate) fn command(name: &str) -> Option<Signature> {
    let (shape, domains) = match name.trim_end_matches('*') {
        // mathtools: style selector followed by math; bracket dimensions stay data.
        "mathclap" | "mathllap" | "mathrlap" | "cramped" | "crampedllap" | "crampedrlap"
        | "crampedclap" => ("om", "UM"),
        "underbracket" | "overbracket" => ("oom", "UUM"),
        "splitfrac" | "splitdfrac" | "accentset" | "underaccent" => ("mm", "MM"),
        "prescript" => ("mmm", "MMM"),
        "crampedsubstack" | "undertilde" => ("m", "M"),
        "mathlarger" | "mathsmaller" => ("m", "M"),
        "cancel" | "bcancel" | "xcancel" => ("m", "I"),
        "cancelto" => ("mm", "MM"),
        "nicefrac" => ("omm", "UII"),
        "sfrac" => ("omom", "UIUI"),
        // Extensible arrows have mathematical upper/lower labels.
        "xleftrightarrow"
        | "xLeftarrow"
        | "xRightarrow"
        | "xLeftrightarrow"
        | "xhookleftarrow"
        | "xhookrightarrow"
        | "xmapsto"
        | "xlongrightarrow"
        | "xlongleftarrow"
        | "xlongleftrightarrow"
        | "xrightarrow"
        | "xleftarrow"
        | "xleftharpoonup"
        | "xleftharpoondown"
        | "xrightharpoonup"
        | "xrightharpoondown"
        | "xrightleftharpoons"
        | "xleftrightharpoons" => ("om", "MM"),
        // commath 0.3: optional derivative order, function, variable.
        "od" | "tod" | "dod" | "pd" | "tpd" | "dpd" => ("omm", "MMM"),
        "md" | "tmd" | "dmd" => ("mmmmmm", "MMMMMM"),
        // tensor 2.2: optional left indices, object, right indices.
        "tensor" => ("omm", "MMM"),
        "indices" => ("m", "M"),
        "nuclide" => ("oom", "MMM"),
        // braket: one mathematical argument, including vertical-bar separators.
        "bra" | "ket" | "braket" | "Bra" | "Ket" | "Braket" | "set" | "Set" => ("m", "M"),
        // Declaration bodies must retain their literal token spelling.
        "DeclarePairedDelimiter" => ("mmm", "UUU"),
        "DeclarePairedDelimiterX" => ("mommm", "UUUUU"),
        "DeclarePairedDelimiterXPP" => ("mommmmm", "UUUUUUU"),
        "reDeclarePairedDelimiterInnerWrapper" => ("mmm", "UUU"),
        _ => return super::scientific::math_signature(name),
    };
    Some(Signature { shape, domains })
}

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &[
    "dcases", "dcases*", "rcases", "rcases*", "drcases", "drcases*", "cases*", "darray",
];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "dcases" | "dcases*" | "rcases" | "rcases*" | "drcases" | "drcases*" | "cases*" => "",
        "dmath" | "dmath*" | "dseries" | "dseries*" | "dgroup" | "dgroup*" | "darray" => "o",
        _ => return super::scientific::header_signature(name),
    })
}

pub(crate) fn is_math_environment(name: &str) -> bool {
    ALIGNMENT_ENVIRONMENTS.contains(&name)
        || super::scientific::ALIGNMENT_ENVIRONMENTS.contains(&name)
        || matches!(name, "dmath" | "dmath*")
}

pub(crate) fn text_case_column(name: &str) -> bool {
    matches!(name, "dcases*" | "rcases*" | "drcases*" | "cases*")
}

pub(crate) fn has_extensions(source: &str) -> bool {
    memchr::memchr_iter(b'\\', source.as_bytes()).any(|start| {
        let Some((name, end)) = crate::syntax::command_at(source, start) else {
            return false;
        };
        command(name).is_some_and(|signature| signature.domains.contains(['M', 'I']))
            || name == "begin"
                && source[end..]
                    .trim_start()
                    .strip_prefix('{')
                    .and_then(|rest| rest.split_once('}'))
                    .is_some_and(|(name, _)| is_math_environment(name))
    })
}
