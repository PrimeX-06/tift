//! Exact AMS command aliases and TeX classes supplement the Unicode baseline.

use badness_formatter::semantic::{MathAtomInfo, MathClass, math_command_info};

pub(crate) fn command_info(name: &str) -> MathAtomInfo {
    let baseline = math_command_info(name);
    let class = match name {
        // TeX AMS declarations can differ from Unicode's primary spelling and
        // class. In particular, AMS declares both dotted comparisons as Bin.
        "centerdot" | "gtrdot" | "lessdot" | "lhd" | "rhd" | "smallsetminus" | "unlhd"
        | "unrhd" | "bmod" => MathClass::Bin,
        "backepsilon" | "because" | "blacktriangleleft" | "blacktriangleright"
        | "circlearrowleft" | "circlearrowright" | "doteqdot" | "gvertneqq" | "lvertneqq"
        | "ngeqq" | "ngeqslant" | "nleqq" | "nleqslant" | "npreceq" | "nshortmid"
        | "nshortparallel" | "nsubseteqq" | "nsucceq" | "nsupseteqq" | "ntriangleleft"
        | "ntriangleright" | "shortmid" | "shortparallel" | "smallfrown" | "smallsmile"
        | "therefore" | "thickapprox" | "thicksim" | "varpropto" | "varsubsetneq"
        | "varsubsetneqq" | "varsupsetneq" | "varsupsetneqq" | "dashleftarrow"
        | "dashrightarrow" | "relbar" | "Relbar" | "And" | "xleftarrow" | "xrightarrow" => {
            MathClass::Rel
        }
        "injlim" | "projlim" | "varinjlim" | "varprojlim" | "varliminf" | "varlimsup"
        | "idotsint" | "qopname" | "sideset" => MathClass::Op,
        "colon" => MathClass::Punct,
        // Exact public aliases from amssymb/amsfonts retain the target's full
        // metadata. These aliases form one hop and never classify name prefixes.
        "restriction" => return command_info("upharpoonright"),
        "Doteq" => return command_info("doteqdot"),
        "doublecup" => return command_info("Cup"),
        "doublecap" => return command_info("Cap"),
        "llless" => return command_info("lll"),
        "gggtr" => return command_info("ggg"),
        "dasharrow" => return command_info("dashrightarrow"),
        "Box" => return command_info("square"),
        "Diamond" => return command_info("lozenge"),
        "leadsto" => return command_info("rightsquigarrow"),
        _ => return baseline,
    };
    MathAtomInfo {
        class,
        delimiter: None,
    }
}


