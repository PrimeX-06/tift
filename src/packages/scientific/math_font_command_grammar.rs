//! Math font declarations and symbol alphabets.

specifications! {
    // bm: optional math version, target control sequence, and stored symbol.
    "DeclareBoldMathCommand" => ("omm", "UUU", "omm"),
    "bmdefine" | "hmdefine" => ("mm", "UU", "mm"),
    "hm" => ("m", "I", ""),
    "unimathsetup" | "addnolimits" | "removenolimits" => ("m", "U", "m"),
    // Font declarations permit another option argument after the font name.
    "setmathfont" => ("omo", "UUU", "omo"),
    "setmathfontface" => ("momo", "UUUU", "momo"),
    "NewNegationCommand" | "RenewNegationCommand" => ("mm", "UU", "mm"),
    // LaTeX math-alphabet declarations store font encodings, families,
    // symbols and slot selectors as literal declaration data.
    "DeclareMathAlphabet" => ("mmmmm", "UUUUU", "mmmmm"),
    "SetMathAlphabet" => ("mmmmmm", "UUUUUU", "mmmmmm"),
    "DeclareSymbolFont" => ("mmmmm", "UUUUU", "mmmmm"),
    "SetSymbolFont" => ("mmmmmm", "UUUUUU", "mmmmmm"),
    "DeclareMathSymbol" | "DeclareMathAccent" => ("mmmm", "UUUU", "mmmm"),
    "DeclareMathDelimiter" => ("mmmmmm", "UUUUUU", "mmmmmm"),
    "DeclareMathRadical" => ("mmmmm", "UUUUU", "mmmmm"),
    "DeclareMathSizes" => ("mmmm", "UUUU", "mmmm"),
    "DeclareMathVersion" => ("m", "U", "m"),
    // unicode-math changes the mathematical operator alphabet by name.
    "setoperatorfont" => ("m", "U", "m"),
    // isomath math alphabets and semantic vector/matrix/tensor wrappers.
    "mathbfit" | "mathsfit" | "mathsfbfit" | "vectorsym" | "matrixsym" | "tensorsym"
        => ("m", "M", ""),
    "mathbold" | "EuScript" | "EuFrak" | "CMcal" => ("m", "M", ""),
    "symup" | "symit" | "symbf" | "symbfit" | "symbfup" | "symsf" | "symtt"
        | "symscr" | "symcal" | "symfrak" | "symbb" | "symsfup" | "symsfit"
        | "symbfsf" | "symbfsfup" | "symbfsfit" | "symbfscr" | "symbfcal"
        | "symbffrak" => ("m", "M", ""),
}
