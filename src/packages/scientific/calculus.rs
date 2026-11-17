//! interval, derivative and diffcoeff mathematical operands and factories.

use super::math;

specifications! {
    "intervalconfig" => ("m", "U", "m"),
    "interval" | "ointerval" | "linterval" | "rinterval" => ("omm", "UMM", "o"),

    // derivative generates commands from these literal xparse signatures.
    "derivset" => ("mo", "UU", "mo"),
    "NewDerivative" | "RenewDerivative" | "ProvideDerivative" | "DeclareDerivative"
        | "NewDifferential" | "RenewDifferential" | "ProvideDifferential"
        | "DeclareDifferential" => ("mmo", "UUU", "mmo"),
    "odv" | "mdv" | "fdv" | "adv" | "jdv" | "pdv"
        => ("s o m t/ t! m !e{_^}", "UUMUUMMM", "s o"),
    "odif" | "mdif" | "fdif" | "adif" | "pdif" => ("s o m", "UUM", "s o"),
    "slashfrac" => ("omm", "UMM", "o"),

    // diffcoeff: variant, two stars, order, settings, two math operands and
    // tightly bound evaluation point; the variant has matching dot delimiters.
    "difdef" => ("mmm", "UUU", "mmm"),
    "diffdef" => ("mm", "UU", "mm"),
    "difoverride" => ("m", "U", "m"),
    "diff" | "diffp" | "difs" | "difsp" | "difc" | "difcp"
        => ("D..{} s s O{1} D<>{} m m !o", "UUUMUMMM", "D..{} s s"),
    "jacob" | "difj" => ("D..{} m m", "UMM", "D..{}"),
    "dl" | "difl" => ("D..{} O{1} m e{^}", "UMMM", "D..{}"),
    "spacedone" | "spacedneg" | "spacednil" => ("", "", ""),
    "negmu" | "nilmu" | "onemu" | "twomu" => ("s", "U", "s"),
}

pub(super) fn factory_math_signature(name: &str) -> Option<math::Signature> {
    let command = match name {
        "NewDerivative" | "RenewDerivative" | "ProvideDerivative" | "DeclareDerivative" => "odv",
        "NewDifferential" | "RenewDifferential" | "ProvideDifferential" | "DeclareDifferential" => {
            "odif"
        }
        _ => return None,
    };
    specification(command).map(super::CommandSpec::math_signature)
}
