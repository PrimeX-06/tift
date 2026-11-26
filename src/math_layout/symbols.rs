//! Exact AMS command aliases and TeX classes supplement the Unicode baseline.

mod unicode;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum MathClass {
    #[default]
    Ord,
    Bin,
    Rel,
    Op,
    Punct,
    Open,
    Close,
    Inner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DelimiterRole {
    Open,
    Close,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MathAtomInfo {
    pub class: MathClass,
    pub delimiter: Option<DelimiterRole>,
}

fn info(class: MathClass) -> MathAtomInfo {
    MathAtomInfo {
        class,
        delimiter: match class {
            MathClass::Open => Some(DelimiterRole::Open),
            MathClass::Close => Some(DelimiterRole::Close),
            _ => None,
        },
    }
}

pub(crate) fn known_command(name: &str) -> bool {
    unicode::COMMANDS
        .binary_search_by_key(&name, |(name, _)| *name)
        .is_ok()
        || command_info(name) != MathAtomInfo::default()
        || matches!(
            name,
            "alpha"
                | "beta"
                | "gamma"
                | "delta"
                | "epsilon"
                | "varepsilon"
                | "zeta"
                | "eta"
                | "theta"
                | "vartheta"
                | "iota"
                | "kappa"
                | "lambda"
                | "mu"
                | "nu"
                | "xi"
                | "pi"
                | "varpi"
                | "rho"
                | "varrho"
                | "sigma"
                | "varsigma"
                | "tau"
                | "upsilon"
                | "phi"
                | "varphi"
                | "chi"
                | "psi"
                | "omega"
                | "dots"
                | "ldots"
                | "dotsb"
                | "dotsc"
                | "dotsi"
                | "dotso"
                | "infty"
                | "not"
        )
}

pub(crate) fn character_info(character: char) -> MathAtomInfo {
    let class = match character {
        '+' | '-' | '*' => MathClass::Bin,
        '=' | '<' | '>' => MathClass::Rel,
        ':' | ',' | ';' => MathClass::Punct,
        '(' | '[' => MathClass::Open,
        ')' | ']' => MathClass::Close,
        _ => unicode::CHARACTERS
            .binary_search_by_key(&character, |(character, _)| *character)
            .map_or(MathClass::Ord, |index| unicode::CHARACTERS[index].1),
    };
    if character == '!' {
        MathAtomInfo {
            class,
            delimiter: None,
        }
    } else {
        info(class)
    }
}

fn baseline_info(name: &str) -> MathAtomInfo {
    let class = match name {
        "le" | "ge" | "ne" | "neq" | "to" | "gets" | "iff" | "implies" | "impliedby" | "owns"
        | "notin" | "colon=" | "doteq" => MathClass::Rel,
        "cdot" | "land" | "lor" | "circ" | "diamond" | "bigtriangledown" => MathClass::Bin,
        "arccos" | "arcsin" | "arctan" | "arg" | "cos" | "cosh" | "cot" | "coth" | "csc"
        | "deg" | "det" | "dim" | "exp" | "gcd" | "hom" | "inf" | "ker" | "lg" | "lim"
        | "liminf" | "limsup" | "ln" | "log" | "max" | "min" | "Pr" | "sec" | "sin" | "sinh"
        | "sup" | "tan" | "tanh" | "iint" | "iiint" | "iiiint" | "operatorname" => MathClass::Op,
        "lvert" | "lVert" | "langle" | "lbrace" | "lbrack" | "ulcorner" | "llcorner" => {
            MathClass::Open
        }
        "rvert" | "rVert" | "rangle" | "rbrace" | "rbrack" | "urcorner" | "lrcorner" => {
            MathClass::Close
        }
        "mathbin" => MathClass::Bin,
        "mathrel" => MathClass::Rel,
        "mathop" => MathClass::Op,
        "mathopen" => MathClass::Open,
        "mathclose" => MathClass::Close,
        "mathpunct" => MathClass::Punct,
        "mathinner" => MathClass::Inner,
        _ => unicode::COMMANDS
            .binary_search_by_key(&name, |(name, _)| *name)
            .map_or(MathClass::Ord, |index| unicode::COMMANDS[index].1),
    };
    info(class)
}

pub(crate) fn command_info(name: &str) -> MathAtomInfo {
    let baseline = baseline_info(name);
    let class = match name {
        // TeX AMS declarations can differ from Unicode's primary spelling and
        // class. In particular, AMS declares both dotted comparisons as Bin.
        "centerdot" | "gtrdot" | "lessdot" | "lhd" | "rhd" | "smallsetminus" | "unlhd"
        | "unrhd" | "bmod" => MathClass::Bin,
        "coloneqq"
        | "Coloneqq"
        | "coloneq"
        | "Coloneq"
        | "eqqcolon"
        | "Eqqcolon"
        | "eqcolon"
        | "Eqcolon"
        | "colonapprox"
        | "Colonapprox"
        | "colonsim"
        | "Colonsim"
        | "approxcolon"
        | "Approxcolon"
        | "simcolon"
        | "Simcolon"
        | "colondash"
        | "Colondash"
        | "dashcolon"
        | "Dashcolon"
        | "xleftrightarrow"
        | "xLeftarrow"
        | "xRightarrow"
        | "xLeftrightarrow"
        | "xhookleftarrow"
        | "xhookrightarrow"
        | "xmapsto"
        | "xlongrightarrow"
        | "xlongleftarrow"
        | "xlongleftrightarrow"
        | "xleftharpoonup"
        | "xleftharpoondown"
        | "xrightharpoonup"
        | "xrightharpoondown"
        | "xrightleftharpoons"
        | "xleftrightharpoons"
        | "backepsilon"
        | "because"
        | "blacktriangleleft"
        | "blacktriangleright"
        | "circlearrowleft"
        | "circlearrowright"
        | "doteqdot"
        | "gvertneqq"
        | "lvertneqq"
        | "ngeqq"
        | "ngeqslant"
        | "nleqq"
        | "nleqslant"
        | "npreceq"
        | "nshortmid"
        | "nshortparallel"
        | "nsubseteqq"
        | "nsucceq"
        | "nsupseteqq"
        | "ntriangleleft"
        | "ntriangleright"
        | "shortmid"
        | "shortparallel"
        | "smallfrown"
        | "smallsmile"
        | "therefore"
        | "thickapprox"
        | "thicksim"
        | "varpropto"
        | "varsubsetneq"
        | "varsubsetneqq"
        | "varsupsetneq"
        | "varsupsetneqq"
        | "dashleftarrow"
        | "dashrightarrow"
        | "relbar"
        | "Relbar"
        | "And"
        | "xleftarrow"
        | "xrightarrow" => MathClass::Rel,
        "injlim" | "projlim" | "varinjlim" | "varprojlim" | "varliminf" | "varlimsup"
        | "idotsint" | "qopname" | "sideset" | "dif" | "Dif" => MathClass::Op,
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
