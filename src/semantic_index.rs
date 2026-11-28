//! Source-ordered declarations, scopes, and semantic lookup tables.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use crate::config::{EnvironmentAlias, EnvironmentArguments, ImportedCommand};
use crate::source_model::{Conditional, Declaration, EnvironmentDeclaration, SyntaxRule};
use crate::syntax;

#[derive(Debug, Clone)]
pub(crate) struct SemanticIndex {
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) environment_declarations: Vec<EnvironmentDeclaration>,
    pub(crate) environment_declaration_names: HashSet<String>,
    pub(crate) environment_arguments: HashMap<usize, EnvironmentArguments>,
    pub(crate) environment_header_starts: HashMap<usize, usize>,
    pub(crate) explicit_wrapper_ends: HashMap<usize, usize>,
    pub(crate) context: crate::packages::context::ContextIndex,
    pub(crate) conditionals: Vec<Conditional>,
    pub(crate) rules: Vec<SyntaxRule>,
    /// Where each spanless builtin rule first entered the ordered registry.
    pub(crate) builtin_first: HashMap<String, usize>,
    pub(crate) boundaries: HashMap<usize, EnvironmentAlias>,
    pub(crate) group_openers: HashSet<usize>,
    pub(crate) inline_math_shifts: HashSet<usize>,
    pub(crate) exported: BTreeMap<String, EnvironmentAlias>,
    pub(crate) exported_unknown: HashSet<String>,
    pub(crate) exported_environments: BTreeMap<String, Option<EnvironmentArguments>>,
    pub(crate) exported_verbatim: BTreeMap<String, Option<crate::config::VerbatimTerminator>>,
    pub(crate) exported_commands: BTreeMap<String, ImportedCommand>,
    pub(crate) registry: HashMap<String, SyntaxRule>,
    pub(crate) invocations: HashMap<usize, SyntaxRule>,
    pub(crate) invocation_arguments: HashMap<usize, Vec<syntax::xparse::ArgumentSlot>>,
    pub(crate) stored_arguments: Vec<Range<usize>>,
    pub(crate) inferred_math_signatures: HashMap<usize, (&'static str, &'static str)>,
    pub(crate) constructed: HashMap<usize, (String, usize)>,
    pub(crate) opaque: Vec<Range<usize>>,
    pub(crate) protected: Vec<Range<usize>>,
    pub(crate) document_class: Option<String>,
}

impl SemanticIndex {
    /// Updates a settled index after a text reader emitted no semantic events.
    pub(crate) fn project_text(&mut self, change: &crate::source_document::SourceChange) {
        fn keys<T>(map: &mut HashMap<usize, T>, change: &crate::source_document::SourceChange) {
            *map = map
                .drain()
                .map(|(at, value)| (change.position(at), value))
                .collect();
        }
        if change.old.end == change.new.end {
            return;
        }
        for declaration in &mut self.declarations {
            change.project(&mut declaration.span);
            change.project(&mut declaration.body);
        }
        for declaration in &mut self.environment_declarations {
            change.project(&mut declaration.span);
            declaration
                .bodies
                .iter_mut()
                .for_each(|body| change.project(body));
        }
        for conditional in &mut self.conditionals {
            change.project(&mut conditional.span);
            conditional
                .branches
                .iter_mut()
                .for_each(|r| change.project(r));
        }
        for rule in self
            .rules
            .iter_mut()
            .chain(self.registry.values_mut())
            .chain(self.invocations.values_mut())
        {
            if let Some(range) = &mut rule.span {
                change.project(range);
            }
        }
        for at in self.builtin_first.values_mut() {
            *at = change.position(*at);
        }
        for range in self
            .stored_arguments
            .iter_mut()
            .chain(&mut self.opaque)
            .chain(&mut self.protected)
        {
            change.project(range);
        }
        for slots in self.invocation_arguments.values_mut() {
            for slot in slots {
                change.project(&mut slot.range);
                change.project(&mut slot.body);
            }
        }
        for end in self
            .environment_header_starts
            .values_mut()
            .chain(self.explicit_wrapper_ends.values_mut())
        {
            *end = change.position(*end);
        }
        for (_, end) in self.constructed.values_mut() {
            *end = change.position(*end);
        }
        keys(&mut self.environment_arguments, change);
        keys(&mut self.environment_header_starts, change);
        keys(&mut self.explicit_wrapper_ends, change);
        keys(&mut self.boundaries, change);
        keys(&mut self.invocations, change);
        keys(&mut self.invocation_arguments, change);
        keys(&mut self.inferred_math_signatures, change);
        keys(&mut self.constructed, change);
        for set in [&mut self.group_openers, &mut self.inline_math_shifts] {
            *set = set.drain().map(|at| change.position(at)).collect();
        }
        self.context.project_positions(change);
    }
}
