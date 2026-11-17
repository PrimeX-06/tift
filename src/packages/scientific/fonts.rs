//! unicode-math font declarations and symbol alphabets.

specifications! {
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
