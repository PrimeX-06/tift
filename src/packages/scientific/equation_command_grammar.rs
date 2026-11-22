//! Equation helpers from amsmath and mathtools.

specifications! {
    // Delimiters, rule thickness and style precede the two operands.
    "genfrac" => ("mmmmmm", "UUUUMM", "mmmm"),
    "Aboxed" => ("m", "M", ""),
    "intertext" | "shortintertext" => ("m", "T", ""),
    "mathtoolsset" => ("m", "U", "m"),
    "mathmbox" => ("m", "M", ""),
    "mathmakebox" => ("oom", "UUM", "oo"),
    "smashoperator" => ("om", "UM", "o"),
    "ArrowBetweenLines" => ("s o", "UM", "s"),
    "MoveEqLeft" => ("o", "U", "o"),
    // amsmath equation numbering controls. Labels and tag formats are literal
    // metadata, while intertext remains printable text above.
    "tag" => ("m", "U", "m"),
    "eqref" => ("m", "U", "m"),
    "refeq" | "noeqref" => ("m", "U", "m"),
    "numberwithin" => ("mm", "UU", "mm"),
    "allowdisplaybreaks" | "displaybreak" => ("o", "U", "o"),
    "raisetag" => ("m", "U", "m"),
    // mathtools tag forms retain all delimiter and format tokens as data.
    "newtagform" | "renewtagform" => ("mmm", "UUU", "mmm"),
    "usetagform" => ("m", "U", "m"),
    // nccmath medium-size math wrappers and fraction helpers.
    "medmath" | "medop" | "medint" => ("m", "M", ""),
    "medintcorr" => ("m", "U", "m"),
    "mfrac" | "mbinom" => ("mm", "MM", ""),
}
