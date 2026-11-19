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
    "symup" | "symit" | "symbf" | "symbfit" | "symbfup" | "symsf" | "symtt"
        | "symscr" | "symcal" | "symfrak" | "symbb" | "symsfup" | "symsfit"
        | "symbfsf" | "symbfsfup" | "symbfsfit" | "symbfscr" | "symbfcal"
        | "symbffrak" => ("m", "M", ""),
}
