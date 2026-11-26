//! siunitx number/unit languages and the physics quantity-name collision.

use super::{CommandSpec, InvocationContext};

specifications! {
    // Signs, uncertainty parentheses, list separators and unit operators are
    // significant tokens in the package input languages.
    "sisetup" => ("m", "U", "m"),
    "num" => ("og", "UU", "og"),
    "numlist" | "numproduct" | "complexnum" | "tablenum"
        | "ang" | "duration" | "unit" | "si" => ("om", "UU", "om"),
    "qtylist" | "qtyproduct" | "complexqty" | "SIlist" => ("omm", "UUU", "omm"),
    "numrange" => ("omm", "UUU", "omm"),
    "qtyrange" | "SIrange" => ("ommm", "UUUU", "ommm"),
    "SI" => ("omom", "UUUU", "omom"),
    "DeclareSIPower" | "DeclareSIPrefix" | "DeclareBinaryPrefix" => ("mmm", "UUU", "mmm"),
    "DeclareSIQualifier" | "DeclareSIPrePower" | "DeclareSIPostPower" => ("mm", "UU", "mm"),
    "DeclareSIUnit" => ("omm", "UUU", "omm"),
    // units.sty: unit values and the numerator/denominator expressions are
    // kept together, with an optional leading value in either command.
    "unitfrac" => ("omm", "UUU", "omm"),
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
    if name == "num" && context.possibly_has_package("siunitx") {
        return Some(CommandSpec::new("om", "UU", "om"));
    }
    if context.has_package("SIunits") {
        match name.trim_end_matches('*') {
            "unit" => return Some(CommandSpec::new("mm", "UU", "mm")),
            "power" => return Some(CommandSpec::new("mm", "UU", "mm")),
            _ => (),
        }
    }
    if context.has_package("physunits")
        && matches!(
            name,
            "V" | "Volt"
                | "Coulomb"
                | "Ohm"
                | "Amp"
                | "Farad"
                | "Henry"
                | "Tesla"
                | "Gauss"
                | "calorie"
                | "Hz"
                | "barP"
                | "Pa"
                | "Ba"
                | "Torr"
                | "Celsius"
                | "Fahrenheit"
                | "Rankine"
                | "Kelvin"
        )
    {
        return Some(CommandSpec::new("o", "U", "o"));
    }
    (name.trim_end_matches('*') == "qty"
        && context.has_package("siunitx")
        && !context.possibly_has_package("physics"))
    .then_some(CommandSpec::new("omm", "UUU", "omm"))
}
