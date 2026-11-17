//! siunitx number/unit languages and the physics quantity-name collision.

use super::{CommandSpec, InvocationContext};

specifications! {
    // Signs, uncertainty parentheses, list separators and unit operators are
    // significant tokens in the package input languages.
    "sisetup" => ("m", "U", "m"),
    "num" | "numlist" | "numproduct" | "complexnum" | "tablenum"
        | "ang" | "duration" | "unit" | "si" => ("om", "UU", "om"),
    "qtylist" | "qtyproduct" | "complexqty" | "SIlist" => ("omm", "UUU", "omm"),
    "numrange" => ("omm", "UUU", "omm"),
    "qtyrange" | "SIrange" => ("ommm", "UUUU", "ommm"),
    "SI" => ("omom", "UUUU", "omom"),
    "DeclareSIPower" | "DeclareSIPrefix" | "DeclareBinaryPrefix" => ("mmm", "UUU", "mmm"),
    "DeclareSIQualifier" | "DeclareSIPrePower" | "DeclareSIPostPower" => ("mm", "UU", "mm"),
    "DeclareSIUnit" => ("omm", "UUU", "omm"),
}

pub(super) fn is_context_metadata(name: &str) -> bool {
    name.trim_end_matches('*') == "qty"
}

pub(super) fn context_ambiguous(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    (name.trim_end_matches('*') == "qty"
        && context.possibly_has_package("siunitx")
        && (!context.has_package("siunitx") || context.possibly_has_package("physics")))
        .then_some("qty has incompatible siunitx and physics argument grammars, or its package loading is conditional")
}

pub(super) fn context_specification(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<CommandSpec> {
    (name.trim_end_matches('*') == "qty"
        && context.has_package("siunitx")
        && !context.possibly_has_package("physics"))
    .then_some(CommandSpec::new("omm", "UUU", "omm"))
}
