//! Mathematical regions and argument domains from tift's structural scanner.

use std::collections::HashSet;
use std::ops::Range;

use crate::syntax::{self, BlockKind, TokenKind};
use crate::{CompiledConfig, FormatError, packages::math};

#[derive(Debug, Clone)]
pub(crate) struct MathRegion {
    pub body: Range<usize>,
    pub display: Option<(usize, String)>,
    pub environment: Option<Range<usize>>,
    pub text_cases: bool,
}

pub(crate) struct MathModes {
    pub regions: Vec<MathRegion>,
    pub declared: HashSet<String>,
}

impl MathModes {
    pub(crate) fn build(
        source: &str,
        config: &CompiledConfig,
        protected: &[Range<usize>],
    ) -> Result<Self, FormatError> {
        let (tokens, blocks) = syntax::blocks_with_ranges(source, config, protected)?;
        let declared = syntax::declared_command_names(source, &config.config)?;
        let mut regions = Vec::new();
        for block in &blocks {
            let special = block.kind == BlockKind::Special
                && matches!(
                    block.name.as_str(),
                    "inlineMath" | "displayMath" | "displayMathTeX"
                );
            let environment = block.kind == BlockKind::Environment
                && !config
                    .config
                    .syntax
                    .environment_arguments
                    .contains_key(&block.name)
                && is_math_environment(&block.name);
            if special || environment {
                let mut body = block.body.clone();
                if environment {
                    let signature = math::header_signature(&block.name).or_else(|| {
                        match block.name.trim_end_matches('*') {
                            "array" | "subarray" => Some("om"),
                            "alignedat" => Some("om"),
                            "alignat" => Some("m"),
                            "aligned" | "gathered" => Some("o"),
                            _ => None,
                        }
                    });
                    if let Some(signature) = signature {
                        let arguments = crate::layout::signature_argument_ranges(
                            source,
                            body.clone(),
                            signature.bytes(),
                        );
                        if let Some(last) = arguments.last() {
                            body.start = last.end
                        }
                    }
                }
                let display = if environment {
                    matches!(
                        block.name.as_str(),
                        "equation" | "equation*" | "displaymath"
                    )
                    .then(|| (block.begin.start, block.name.clone()))
                } else {
                    (block.name != "inlineMath").then(|| {
                        (
                            block.begin.start,
                            if block.name == "displayMathTeX" {
                                "$$"
                            } else {
                                "displayMath"
                            }
                            .into(),
                        )
                    })
                };
                regions.push(MathRegion {
                    body,
                    display,
                    environment: environment.then_some(block.begin.start..block.end.end),
                    text_cases: environment && math::text_case_column(&block.name),
                });
            }
        }
        for token in &tokens {
            if token.kind != TokenKind::Command {
                continue;
            }
            let Some((name, end)) = syntax::command_at(source, token.span.start) else {
                continue;
            };
            let name = name.trim_end_matches('*');
            if declared.contains(name) || config.config.syntax.commands.contains_key(name) {
                continue;
            }
            let Some(signature) = command_signature(name) else {
                continue;
            };
            for (range, domain) in argument_ranges(source, end, source.len(), signature) {
                if domain == b'M' {
                    regions.push(MathRegion {
                        body: range.start + 1..range.end - 1,
                        display: None,
                        environment: None,
                        text_cases: false,
                    });
                }
            }
        }
        regions.sort_by_key(|region| (region.body.start, std::cmp::Reverse(region.body.end)));
        regions.dedup_by(|left, right| left.body == right.body);
        Ok(Self { regions, declared })
    }
}

pub(crate) fn is_math_environment(name: &str) -> bool {
    math::is_math_environment(name)
        || crate::config::matrix_environment(name)
        || matches!(
            name.trim_end_matches('*'),
            "math"
                | "displaymath"
                | "equation"
                | "align"
                | "alignat"
                | "aligned"
                | "alignedat"
                | "flalign"
                | "gather"
                | "gathered"
                | "multline"
                | "multlined"
                | "split"
                | "eqnarray"
                | "array"
                | "subarray"
                | "cases"
        )
}

pub(crate) fn command_signature(name: &str) -> Option<math::Signature> {
    if let Some(signature) = math::command(name) {
        return Some(signature);
    }
    let (shape, domains) = match name.trim_end_matches('*') {
        "frac" | "dfrac" | "tfrac" | "binom" | "dbinom" | "tbinom" | "overset" | "underset"
        | "stackrel" => ("mm", "MM"),
        "mathchoice" => ("mmmm", "MMMM"),
        "sqrt" => ("om", "MM"),
        "ensuremath" | "boxed" | "substack" | "mathbin" | "mathrel" | "mathop" | "mathord"
        | "mathopen" | "mathclose" | "mathpunct" | "mathinner" => ("m", "M"),
        "mathrm" | "mathit" | "mathbf" | "mathsf" | "mathtt" | "mathnormal" | "mathcal"
        | "mathbb" | "mathfrak" | "mathscr" | "boldsymbol" | "bm" | "pmb" | "hat" | "widehat"
        | "check" | "breve" | "acute" | "grave" | "tilde" | "widetilde" | "bar" | "vec" | "dot"
        | "ddot" | "dddot" | "ddddot" | "overline" | "underline" | "overbrace" | "underbrace"
        | "overrightarrow" | "overleftarrow" | "overleftrightarrow" | "underrightarrow"
        | "underleftarrow" | "vphantom" | "hphantom" | "phantom" | "smash" => ("m", "I"),
        "text" | "textrm" | "textsf" | "texttt" | "textnormal" | "textbf" | "textit" | "textmd"
        | "textup" | "textsl" | "emph" | "mbox" | "hbox" | "operatorname" => ("m", "U"),
        "qopname" => ("mmm", "UUU"),
        "sideset" => ("mmm", "MMM"),
        "label" | "tag" | "ref" | "eqref" => ("m", "U"),
        "hspace" | "vspace" | "kern" | "mkern" => ("m", "U"),
        _ => return None,
    };
    Some(math::Signature { shape, domains })
}

/// Locate positional groups without consuming following operands of zero-arg symbols.
pub(crate) fn argument_ranges(
    source: &str,
    mut cursor: usize,
    limit: usize,
    signature: math::Signature,
) -> Vec<(Range<usize>, u8)> {
    let mut ranges = Vec::new();
    for (slot, &shape) in signature.shape.as_bytes().iter().enumerate() {
        while cursor < limit && source.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1
        }
        let (open, close) = if shape == b'o' {
            (b'[', b']')
        } else {
            (b'{', b'}')
        };
        if source.as_bytes().get(cursor) != Some(&open) {
            if shape == b'o' {
                continue;
            }
            break;
        }
        let Some(end) =
            syntax::balanced_end(source, cursor, open, close).filter(|end| *end <= limit)
        else {
            break;
        };
        ranges.push((cursor..end, signature.domains.as_bytes()[slot]));
        cursor = end;
    }
    ranges
}


