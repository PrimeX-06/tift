//! Raw pseudocode bodies from the `algorithms` and `algorithmicx` packages.

pub(crate) const VERBATIM_ENVIRONMENTS: &[&str] = &[
    // algorithms/algorithmic.sty:123 and algorithmicx/algorithmicx.sty:74
    // both define `algorithmic` with one optional line-number frequency.
    // algpseudocode.sty loads algorithmicx and supplies the pseudocode commands
    // used inside this environment (algpseudocode.sty:28--29, 51--81).
    // algorithms/algorithm.sty:80--96 declares the `algorithm` float.
    // That float and algorithm2e environments stay on their own structural
    // paths; algorithm2e formatting remains in code_layout.rs.
    "algorithmic",
];
