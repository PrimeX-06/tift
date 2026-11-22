//! Modular matrix commands from physics2.

specifications! {
    "usephysicsmodule" => ("omo", "UUU", "omo"),
    // physics3 uses its own module loader and namespaced double-stroke
    // identity commands, separate from physics2's package interface.
    "usephxmodu" => ("omo", "UUU", "omo"),
    // physics2's ab module offers fixed-pair delimiter wrappers. A star
    // selects default sizing, while the optional size token remains data.
    "pab" | "bab" | "Bab" | "aab" | "vab" | "Vab" => ("s o m", "UUM", "s o"),
    // diagmat reads mathematical diagonal entries. xmat expands a template
    // with row and column selectors, whose token spelling is significant.
    "diagmat" | "pdiagmat" | "bdiagmat" | "Bdiagmat" | "vdiagmat" | "Vdiagmat"
        => ("om", "UM", "o"),
    "xmat" | "pxmat" | "bxmat" | "Bxmat" | "vxmat" | "Vxmat"
        => ("ommm", "UUUU", "ommm"),
}

pub(super) fn context_specification(
    name: &str,
    context: &super::InvocationContext<'_>,
) -> Option<super::CommandSpec> {
    match name.trim_end_matches('*') {
        "identity" | "Identity" if context.has_package("physics3") => {
            Some(super::CommandSpec::new("o", "U", "o"))
        }
        _ => None,
    }
}
