//! Inference rules and proof trees, including scoped statement names.

use super::{CommandSpec, InvocationContext};

specifications! {
    // mathpartir's starred form uses keys in place of the rule name.
    "inferrule" => ("s o m m", "UUMM", "s o"),
    "mprset" => ("m", "U", "m"),
    "TirName" | "LeftTirName" | "RightTirName" | "LabTirName" | "DefTirName"
        | "RefTirName" => ("m", "T", ""),

    // ebproof exports prefixed statements globally and short names locally.
    "ebproofnewrulestyle" | "ebproofnewstyle" => ("mm", "UU", "mm"),
    "ebproofset" | "ebproofrewrite" => ("m", "U", "m"),
    "ebproofhypo" => ("om", "UM", "o"),
    "ebproofinfer" => ("omom", "UUTM", "om"),
    "ebproofellipsis" => ("mm", "TM", ""),
    "ebproofdelims" => ("mm", "MM", ""),
    "ebproofoverlay" => ("", "", ""),

    // C statements are text boxes; dollar statements contain two math operands.
    "AxiomC" | "UnaryInfC" | "BinaryInfC" | "TrinaryInfC" | "QuaternaryInfC"
        | "QuinaryInfC" | "LeftLabel" | "RightLabel" => ("m", "T", ""),
    "Axiom" | "UnaryInf" | "BinaryInf" | "TrinaryInf" | "QuaternaryInf"
        | "QuinaryInf" => ("r$\\fCenter u{$}", "MM", ""),
    "kernHyps" | "insertBetweenHyps" => ("m", "U", "m"),
    "EnableBpAbbreviations" | "DisplayProof" | "doubleLine" | "alwaysDoubleLine"
        | "singleLine" | "alwaysSingleLine" | "noLine" | "alwaysNoLine"
        | "solidLine" | "alwaysSolidLine" | "dottedLine" | "alwaysDottedLine"
        | "dashedLine" | "alwaysDashedLine" | "rootAtBottom" | "alwaysRootAtBottom"
        | "rootAtTop" | "alwaysRootAtTop" => ("", "", ""),
}

pub(super) fn mathpartir_environment(name: &str) -> bool {
    matches!(name, "mathpar" | "mathparpagebreakable")
}

pub(super) fn is_context_metadata(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "set" | "infer" | "hypo" | "ellipsis" | "rewrite"
    )
}

fn ebproof_context(context: &InvocationContext<'_>) -> bool {
    context.has_package("ebproof")
        && !context.possibly_has_package("bussproofs")
        && !context.possibly_has_package("proof")
        && context.environment() == Some("prooftree")
}

pub(super) fn context_ambiguous(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    let name = name.trim_end_matches('*');
    (matches!(
        name,
        "prooftree" | "set" | "hypo" | "infer" | "ellipsis" | "rewrite" | "delims" | "overlay"
    ) && (name == "prooftree" || context.environment() == Some("prooftree"))
        && (context.possibly_has_package("ebproof") || context.possibly_has_package("bussproofs"))
        && !(context.has_package("ebproof")
            && !context.possibly_has_package("bussproofs")
            && !context.possibly_has_package("proof")
            || context.has_package("bussproofs")
                && !context.possibly_has_package("ebproof")
                && !context.possibly_has_package("proof")))
    .then_some("prooftree has incompatible package grammars, or its package loading is conditional")
}

pub(super) fn context_specification(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<CommandSpec> {
    let global = match name.trim_end_matches('*') {
        "set" => "ebproofset",
        "rewrite" => "ebproofrewrite",
        "hypo" => "ebproofhypo",
        "infer" => "ebproofinfer",
        "ellipsis" => "ebproofellipsis",
        "delims" => "ebproofdelims",
        "overlay" => "ebproofoverlay",
        _ => return None,
    };
    ebproof_context(context)
        .then(|| specification(global))
        .flatten()
}

pub(super) fn header_signature(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    if name != "prooftree" {
        return None;
    }
    if context.has_package("ebproof")
        && !context.possibly_has_package("bussproofs")
        && !context.possibly_has_package("proof")
    {
        Some("s o")
    } else if context.has_package("bussproofs")
        && !context.possibly_has_package("ebproof")
        && !context.possibly_has_package("proof")
    {
        Some("")
    } else {
        None
    }
}
