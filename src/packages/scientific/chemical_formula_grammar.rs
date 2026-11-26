//! Chemical reaction syntax and molecular diagrams.

use super::{CommandSpec, InvocationContext};

specifications! {
    // mhchem 4: the split specification and reaction are both literal input.
    "cesplit" => ("mm", "UU", "mm"),
    "mhchemoptions" => ("m", "U", "m"),

    // chemformula sanitizes formulas before parsing bonds and reaction arrows.
    "ch" => ("og", "UU", "og"),
    "chcpd" => ("om", "UU", "om"),
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
    // chemnum compound IDs are stored labels, not printable TeX formulas.
    // The star defines a label, plus refers to an existing label.
    "cmpd" => ("s t+ o m", "UUUU", "s t+ o"),
    "refcmpd" | "labelcmpd" | "initcmpd" => ("om", "UU", "om"),
    "replacecmpd" => ("s t+ o m", "UUUU", "s t+ o"),
    "cmpdplain" => ("m", "U", "m"),
    "subcmpdplain" | "submaincmpdplain" => ("mm", "UU", "mm"),
    "cmpdproperty" => ("mm", "UU", "mm"),
    "subcmpdproperty" => ("mmm", "UUU", "mmm"),
    "setchemnum" => ("m", "U", "m"),
    // elements.sty queries and declares names, symbols, distributions and
    // isotope lists using atomic numbers or element identifiers. The short
    // \Z command is context-sensitive and is registered below.
    "elementname" | "elementsymbol" | "atomicnumber" | "elconf"
    | "mainelementisotope" => ("m", "U", "m"),
    "setatomname" | "DeclareAtomName" => ("omm", "UUU", "omm"),
    "saveelementname" | "saveelementsymbol" | "saveatomicnumber"
    | "saveelementisotopes" | "savemainelementisotope" => ("mm", "UU", "mm"),
    "setatomsymbol" | "DeclareAtomSymbol" | "setelectrondistribution"
    | "DeclareElectronDistribution" | "setatomisotopes" | "DeclareAtomIsotopes"
        => ("mm", "UU", "mm"),
}

pub(super) fn is_context_metadata(name: &str) -> bool {
    matches!(name.trim_end_matches('*'), "Z" | "ce")
}

pub(super) fn context_specification(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<CommandSpec> {
    if name == "ch" && context.possibly_has_package("chemformula") {
        return Some(CommandSpec::new("om", "UU", "om"));
    }
    // Preserve a possible mhchem argument even when its load is conditional.
    // The short command name has no global interpretation outside that context.
    let package = match name.trim_end_matches('*') {
        "ce" => "mhchem",
        "Z" => "elements",
        _ => return None,
    };
    context
        .possibly_has_package(package)
        .then_some(CommandSpec::new("m", "U", "m"))
}
