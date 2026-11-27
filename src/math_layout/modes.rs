//! Mathematical regions and argument domains from tift's structural scanner.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
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

struct MathInvocation {
    signature: math::Signature,
    arguments: Vec<MathArgument>,
}

pub(crate) struct MathModes {
    pub regions: Vec<MathRegion>,
    pub declared: HashSet<String>,
    source_overrides: HashSet<usize>,
    invocations: HashMap<usize, MathInvocation>,
    environments: HashMap<usize, usize>,
    literal_environments: HashMap<usize, Range<usize>>,
}

impl MathModes {
    pub(super) fn remap(&mut self, positions: &super::PositionMap) -> Option<()> {
        for region in &mut self.regions {
            positions.remap_range(&mut region.body)?;
            if let Some((at, _)) = &mut region.display {
                *at = positions.position(*at, true)?;
            }
            if let Some(range) = &mut region.environment {
                positions.remap_range(range)?;
            }
        }
        self.environments = self
            .environments
            .drain()
            .map(|(at, index)| Some((positions.position(at, true)?, index)))
            .collect::<Option<_>>()?;
        self.literal_environments = self
            .literal_environments
            .drain()
            .map(|(at, mut range)| {
                positions.remap_range(&mut range)?;
                Some((positions.position(at, true)?, range))
            })
            .collect::<Option<_>>()?;
        self.source_overrides = self
            .source_overrides
            .iter()
            .map(|&at| positions.position(at, true))
            .collect::<Option<_>>()?;
        self.invocations = self
            .invocations
            .drain()
            .map(|(at, mut invocation)| {
                for argument in &mut invocation.arguments {
                    positions.remap_range(&mut argument.range)?;
                    positions.remap_range(&mut argument.body)?;
                }
                Some((positions.position(at, true)?, invocation))
            })
            .collect::<Option<_>>()?;
        Some(())
    }

    pub(super) fn environment_at(&self, at: usize, limit: usize) -> Option<&MathRegion> {
        let region = &self.regions[*self.environments.get(&at)?];
        region
            .environment
            .as_ref()
            .filter(|range| range.end <= limit)
            .map(|_| region)
    }

    pub(super) fn literal_environment_end(&self, at: usize, limit: usize) -> Option<usize> {
        self.literal_environments
            .get(&at)
            .filter(|range| range.end <= limit)
            .map(|range| range.end)
    }

    pub(crate) fn signature_at(&self, name: &str, at: usize) -> Option<math::Signature> {
        if self.source_overrides.contains(&at) {
            return None;
        }
        self.invocations
            .get(&at)
            .map(|invocation| invocation.signature)
            .or_else(|| command_signature(name))
    }

    pub(crate) fn is_overridden(&self, name: &str, at: usize) -> bool {
        self.source_overrides.contains(&at)
            || self.declared.contains(name) && !self.invocations.contains_key(&at)
    }

    pub(crate) fn arguments_at(
        &self,
        source: &str,
        at: usize,
        end: usize,
        limit: usize,
        signature: math::Signature,
    ) -> Cow<'_, [MathArgument]> {
        if let Some(invocation) = self.invocations.get(&at) {
            let arguments = &invocation.arguments;
            let count = arguments.partition_point(|argument| argument.range.end <= limit);
            return Cow::Borrowed(&arguments[..count]);
        }
        Cow::Owned(argument_ranges(
            source,
            invocation_cursor(source, end, signature),
            limit,
            signature,
        ))
    }
    pub(crate) fn build(
        source: &str,
        config: &CompiledConfig,
        protected: &[Range<usize>],
        parsing: &mut crate::parse_context::ParseContext<'_>,
    ) -> Result<Self, FormatError> {
        let same_protection = parsing.snapshot(source, false, false)?.protected == protected;
        let parsed = parsing.snapshot(source, same_protection, same_protection)?;
        let exposed_model = (!same_protection).then(|| {
            parsed
                .metadata
                .exposed_model(source, &config.config, protected)
        });
        let analysis = if let Some(model) = &exposed_model {
            crate::source_analysis::SourceAnalysis::for_model(
                source,
                &config.config,
                model.protected_ranges(),
                model,
            )
        } else {
            parsed.analysis(&config.config)
        };
        let adjusted;
        let (tokens, blocks) = if same_protection {
            (
                parsed.tokens.as_deref().expect("math tokens"),
                parsed.blocks.as_deref().expect("math blocks"),
            )
        } else {
            let ranges = exposed_model
                .as_ref()
                .map_or(protected, |model| model.protected_ranges());
            adjusted = syntax::shared_blocks_with_analysis(source, config, ranges, &analysis)?;
            (adjusted.0.as_slice(), adjusted.1.as_slice())
        };
        let declared = analysis.declared_commands().clone();
        let mut regions = Vec::new();
        let mut invocations = HashMap::new();
        let mut source_overrides = HashSet::new();
        let mut literal_environments = HashMap::new();
        for block in blocks {
            // Tabular cells start in text mode even inside a display. Their
            // explicit inner math shifts are independent regions below.
            if block.kind == BlockKind::Environment
                && (block.name == "tikzcd"
                    || crate::tikz_scan::environment(&block.name)
                    || matches!(
                        block.name.as_str(),
                        "tabular" | "tabular*" | "tabularx" | "tabulary" | "longtable"
                    ) && !config
                        .config
                        .syntax
                        .environment_arguments
                        .contains_key(&*block.name))
            {
                literal_environments.insert(block.begin.start, block.begin.start..block.end.end);
                continue;
            }
            let special = block.kind == BlockKind::Special
                // Recovery can terminate several open math shifts at the
                // same protected boundary. Only a paired shift owns a formula.
                && !block.end.is_empty()
                && matches!(
                    block.name.as_str(),
                    "inlineMath" | "displayMath" | "displayMathTeX"
                );
            let environment = block.kind == BlockKind::Environment
                && !config
                    .config
                    .syntax
                    .environment_arguments
                    .contains_key(block.name.as_str())
                && is_math_environment(&block.name)
                && block.name != "tikzcd";
            if special || environment {
                let mut body = block.body.clone();
                if environment {
                    let signature = crate::additional_package_syntax::header_signature(&block.name)
                        .or_else(|| math::header_signature(&block.name))
                        .or_else(|| match block.name.trim_end_matches('*') {
                            "array" | "subarray" => Some("om"),
                            "alignedat" => Some("om"),
                            "alignat" => Some("m"),
                            "aligned" | "gathered" => Some("o"),
                            _ => None,
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
                    .then(|| (block.begin.start, block.name.to_string()))
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
        let model = analysis.model();
        for token in tokens {
            if token.kind != TokenKind::Command {
                continue;
            }
            let Some((name, end)) = model
                .control_at(token.span.start)
                .or_else(|| syntax::command_at(source, token.span.start))
            else {
                continue;
            };
            let name = name.trim_end_matches('*');
            let inferred_signature = model.inferred_math_signature_at(token.span.start);
            let source_override = model
                .invocation_rule(token.span.start)
                .is_some_and(|rule| rule.origin != crate::source_model::RuleOrigin::Builtin);
            if config.config.syntax.commands.contains_key(name)
                || source_override && inferred_signature.is_none()
            {
                source_overrides.insert(token.span.start);
                continue;
            }
            let signature = inferred_signature.or_else(|| {
                crate::packages::context_math_signature(
                    name,
                    &model.invocation_context_at(token.span.start),
                )
                .or_else(|| command_signature(name))
            });
            let Some(signature) = signature else {
                continue;
            };
            let end = invocation_cursor(source, end, signature);
            let invocation_arguments =
                if let Some(slots) = model.invocation_argument_ranges_at(token.span.start) {
                    slots
                        .iter()
                        .filter_map(|slot| {
                            Some(MathArgument {
                                range: slot.range.clone(),
                                body: slot.body.clone(),
                                domain: *signature.domains.as_bytes().get(slot.slot)?,
                            })
                        })
                        .collect()
                } else {
                    argument_ranges(source, end, source.len(), signature)
                };
            for argument in &invocation_arguments {
                if argument.domain == b'M' {
                    regions.push(MathRegion {
                        body: argument.body.clone(),
                        display: None,
                        environment: None,
                        text_cases: false,
                    });
                }
            }
            invocations.insert(
                token.span.start,
                MathInvocation {
                    signature,
                    arguments: invocation_arguments,
                },
            );
        }
        regions.sort_by_key(|region| (region.body.start, std::cmp::Reverse(region.body.end)));
        regions.dedup_by(|left, right| left.body == right.body);
        let mut environments = HashMap::new();
        for (index, region) in regions.iter().enumerate() {
            if let Some(range) = &region.environment {
                environments.entry(range.start).or_insert(index);
            }
        }
        Ok(Self {
            regions,
            declared,
            invocations,
            source_overrides,
            environments,
            literal_environments,
        })
    }
}

pub(crate) fn is_math_environment(name: &str) -> bool {
    math::is_math_environment(name)
        || matches!(name, "empheq" | "numcases" | "subnumcases")
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

#[derive(Clone)]
pub(crate) struct MathArgument {
    pub range: Range<usize>,
    pub body: Range<usize>,
    pub domain: u8,
}

pub(crate) fn invocation_cursor(source: &str, end: usize, signature: math::Signature) -> usize {
    syntax::arguments::invocation_cursor(source, end, signature.shape)
}

/// Locate all argument kinds through the shared signature parser.
pub(crate) fn argument_ranges(
    source: &str,
    cursor: usize,
    limit: usize,
    signature: math::Signature,
) -> Vec<MathArgument> {
    let Some(shape) = syntax::xparse::Signature::parse(signature.shape) else {
        return Vec::new();
    };
    shape
        .slot_ranges(source, cursor..limit)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|argument| {
            Some(MathArgument {
                range: argument.range,
                body: argument.body,
                domain: *signature.domains.as_bytes().get(argument.slot)?,
            })
        })
        .collect()
}
