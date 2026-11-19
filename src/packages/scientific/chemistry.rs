//! Chemical reaction syntax and molecular diagrams.

use super::{CommandSpec, InvocationContext};

specifications! {
    // mhchem 4: the split specification and reaction are both literal input.
    "cesplit" => ("mm", "UU", "mm"),
    "mhchemoptions" => ("m", "U", "m"),

    // chemformula sanitizes formulas before parsing bonds and reaction arrows.
    "ch" | "chcpd" => ("om", "UU", "om"),
    "chname" => ("R(){}R(){}", "UU", "R(){}R(){}"),
    "chlewis" => ("omm", "UUU", "omm"),
    "chstoich" | "bond" | "ShowChemBond" | "setchemformula" => ("m", "U", "m"),
    "NewChemBond" | "ProvideChemBond" | "DeclareChemBond" | "RenewChemBond"
        | "NewChemBondAlias" | "DeclareChemBondAlias"
        | "NewChemSymbol" | "ProvideChemSymbol" | "DeclareChemSymbol" | "RenewChemSymbol"
        => ("mm", "UU", "mm"),
    "NewChemAdditionSymbol" | "ProvideChemAdditionSymbol"
        | "DeclareChemAdditionSymbol" | "RenewChemAdditionSymbol" => ("mmm", "UUU", "mmm"),

    // Molecular/TikZ code is literal. Atom mathematics and caption text remain
    // separate domains available to ordinary nested formatting.
    "chemfig" | "chemmove" => ("om", "UU", "om"),
    "setchemfig" | "setcharge" => ("m", "U", "m"),
    "charge" | "Charge" => ("mm", "UM", "m"),
    "chemname" => ("s o m m", "UUTT", "s o"),
    "chemnameinit" => ("m", "T", ""),
    "chemabove" | "chembelow" | "Chemabove" | "Chembelow" => ("omm", "UMM", "o"),
    "chargedot" | "chargeddot" | "chargerect" | "chargeline" => ("o", "U", "o"),
}

pub(super) fn is_context_metadata(name: &str) -> bool {
    name.trim_end_matches('*') == "ce"
}

pub(super) fn context_specification(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<CommandSpec> {
    // Preserve a possible mhchem argument even when its load is conditional.
    // The short command name has no global interpretation outside that context.
    (is_context_metadata(name) && context.possibly_has_package("mhchem"))
        .then_some(CommandSpec::new("m", "U", "m"))
}
