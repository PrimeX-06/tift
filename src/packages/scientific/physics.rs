//! Modular matrix commands from physics2.

specifications! {
    "usephysicsmodule" => ("omo", "UUU", "omo"),
    // diagmat reads mathematical diagonal entries. xmat expands a template
    // with row and column selectors, whose token spelling is significant.
    "diagmat" | "pdiagmat" | "bdiagmat" | "Bdiagmat" | "vdiagmat" | "Vdiagmat"
        => ("om", "UM", "o"),
    "xmat" | "pxmat" | "bxmat" | "Bxmat" | "vxmat" | "Vxmat"
        => ("ommm", "UUUU", "ommm"),
}
