//! Scientific argument grammars, dispatched by subject.

use crate::{
    config::CommandRule,
    packages::{InvocationContext, math},
    syntax::{arguments::signature_end, xparse::ArgumentSlot},
};
use std::{collections::BTreeMap, ops::Range};

/// U = literal data, M = mathematics, T = text, I = inherited mode.
#[derive(Clone, Copy)]
struct CommandSpec {
    shape: &'static str,
    domains: &'static str,
    data_prefix: &'static str,
}

impl CommandSpec {
    const EMPTY: Self = Self::new("", "", "");

    const fn new(shape: &'static str, domains: &'static str, data_prefix: &'static str) -> Self {
        Self {
            shape,
            domains,
            data_prefix,
        }
    }

    fn math_signature(self) -> math::Signature {
        math::Signature {
            shape: self.shape,
            domains: self.domains,
        }
    }

    fn data_ranges(self, source: &str, cursor: usize) -> Option<Vec<Range<usize>>> {
        crate::syntax::arguments::domain_ranges(source, cursor, self.shape, self.domains)
    }
}

macro_rules! specifications {
    ($( $($name:literal)|+ => ($shape:literal, $domains:literal, $prefix:literal) ),+ $(,)?) => {
        macro_rules! command_names {
            () => { $( $($name)|+ )|+ };
        }
        pub(super) use command_names;

        pub(super) fn specification(name: &str) -> Option<super::CommandSpec> {
            Some(match crate::syntax::unstarred(name) {
                $( $($name)|+ => super::CommandSpec::new($shape, $domains, $prefix), )+
                _ => return None,
            })
        }

        pub(super) fn add_command_rules(commands: &mut std::collections::BTreeMap<String, crate::config::CommandRule>) {
            $( for name in [$($name),+] {
                commands.insert(name.into(), crate::config::CommandRule {
                    signature: Some($shape.into()),
                    ..Default::default()
                });
            } )+
        }
    };
}

#[path = "equation_arrow_grammar.rs"]
mod equation_arrow_grammar;
use equation_arrow_grammar as arrows;
#[path = "calculus_command_grammar.rs"]
mod calculus_command_grammar;
use calculus_command_grammar as calculus;
#[path = "chemical_formula_grammar.rs"]
mod chemical_formula_grammar;
use chemical_formula_grammar as chemistry;
#[path = "equation_command_grammar.rs"]
mod equation_command_grammar;
use equation_command_grammar as equations;
#[path = "math_font_command_grammar.rs"]
mod math_font_command_grammar;
use math_font_command_grammar as fonts;
#[path = "physics_command_grammar.rs"]
mod physics_command_grammar;
use physics_command_grammar as physics;
#[path = "proof_command_grammar.rs"]
mod proof_command_grammar;
use proof_command_grammar as proofs;
#[path = "unit_command_grammar.rs"]
mod unit_command_grammar;
use unit_command_grammar as units;

macro_rules! domains {
    ($($module:ident),+ $(,)?) => {
        fn specification(name: &str) -> Option<CommandSpec> {
            let name = crate::syntax::unstarred(name);
            match name {
                $( $module::command_names!() => $module::specification(name), )+
                _ => None,
            }
        }

        pub(crate) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
            $( $module::add_command_rules(commands); )+
        }
    };
}

domains!(
    units, chemistry, proofs, calculus, fonts, arrows, equations, physics
);

fn contextual_specification(name: &str, context: &InvocationContext<'_>) -> Option<CommandSpec> {
    if context_ambiguous(name, context).is_some() {
        return Some(CommandSpec::EMPTY);
    }
    units::context_specification(name, context)
        .or_else(|| chemistry::context_specification(name, context))
        .or_else(|| physics::context_specification(name, context))
        .or_else(|| proofs::context_specification(name, context))
        .or_else(|| arrows::context_specification(name, context))
}

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = arrows::ALIGNMENT_ENVIRONMENTS;

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    arrows::header_signature(name).or_else(|| proofs::mathpartir_environment(name).then_some("o"))
}

pub(crate) fn is_math_environment(name: &str) -> bool {
    ALIGNMENT_ENVIRONMENTS.contains(&name) || proofs::mathpartir_environment(name)
}

pub(crate) fn environment_for_command(name: &str) -> Option<&str> {
    arrows::environment_for_command(name)
}

pub(crate) fn math_signature(name: &str) -> Option<math::Signature> {
    specification(name).map(CommandSpec::math_signature)
}

pub(crate) fn factory_math_signature(name: &str) -> Option<math::Signature> {
    calculus::factory_math_signature(name)
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    specification(name).is_some_and(|specification| specification.domains.contains('U'))
        || units::is_context_metadata(name)
        || chemistry::is_context_metadata(name)
        || proofs::is_context_metadata(name)
        || arrows::is_context_metadata(name)
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    let specification = specification(name)?;
    signature_end(source, cursor, specification.data_prefix).filter(|&end| end > cursor)
}

pub(crate) fn command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
) -> Option<Vec<Range<usize>>> {
    specification(name)?.data_ranges(source, cursor)
}

pub(crate) fn context_ambiguous(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    units::context_ambiguous(name, context).or_else(|| proofs::context_ambiguous(name, context))
}

pub(crate) fn context_command_signature(
    _source: &str,
    name: &str,
    _cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    contextual_specification(name, context).map(|specification| specification.shape)
}

pub(crate) fn context_command_data_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<Vec<Range<usize>>> {
    if let Some(ranges) = arrows::command_data_ranges(source, name, cursor, context) {
        return Some(ranges);
    }
    contextual_specification(name, context)?.data_ranges(source, cursor)
}

pub(crate) fn context_invocation_ranges(
    source: &str,
    name: &str,
    cursor: usize,
    context: &InvocationContext<'_>,
) -> Option<Result<Vec<ArgumentSlot>, Range<usize>>> {
    arrows::invocation_ranges(source, name, cursor, context)
}

pub(crate) fn context_math_signature(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<math::Signature> {
    contextual_specification(name, context).map(CommandSpec::math_signature)
}

pub(crate) fn context_header_signature(
    name: &str,
    context: &InvocationContext<'_>,
) -> Option<&'static str> {
    proofs::header_signature(name, context)
}
