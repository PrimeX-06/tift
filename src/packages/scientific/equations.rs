//! Equation helpers from amsmath and mathtools.

specifications! {
    // Delimiters, rule thickness and style precede the two operands.
    "genfrac" => ("mmmmmm", "UUUUMM", "mmmm"),
    "intertext" | "shortintertext" => ("m", "T", ""),
    "mathtoolsset" => ("m", "U", "m"),
    "mathmbox" => ("m", "M", ""),
    "mathmakebox" => ("oom", "UUM", "oo"),
    "smashoperator" => ("om", "UM", "o"),
    "ArrowBetweenLines" => ("s o", "UM", "s"),
    "MoveEqLeft" => ("o", "U", "o"),
}
