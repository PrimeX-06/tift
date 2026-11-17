//! chemformula reaction syntax and chemfig molecular diagrams.

specifications! {
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
