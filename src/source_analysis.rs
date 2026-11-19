//! Source-dependent environment metadata scoped to one immutable text snapshot.

use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use crate::Config;
use crate::config::EnvironmentArguments;

pub(crate) struct SourceAnalysis<'a> {
    source: &'a str,
    config: &'a Config,
    protected: Option<&'a [Range<usize>]>,
    memo: Rc<AnalysisMemo>,
    model_ref: Option<&'a crate::source_model::SourceModel>,
}

#[derive(Default)]
pub(crate) struct AnalysisMemo {
    model: OnceCell<crate::source_model::SourceModel>,
    exposed_model: RefCell<Option<ExposedModelCache>>,
    declaration_tokens: OnceCell<bool>,
    boxes: OnceCell<HashMap<String, EnvironmentArguments>>,
    arguments: OnceCell<HashMap<String, EnvironmentArguments>>,
    definitions: OnceCell<Vec<crate::syntax::MacroDefinition>>,
    commands: OnceCell<HashSet<String>>,
    environments: OnceCell<HashSet<String>>,
    visual_headers: OnceCell<Option<HashMap<usize, bool>>>,
    lists: OnceCell<Vec<String>>,
}

struct ExposedModelCache {
    ranges: Vec<Range<usize>>,
    model: Rc<crate::source_model::SourceModel>,
}

impl AnalysisMemo {
    pub(crate) fn with_model(model: crate::source_model::SourceModel) -> Self {
        Self {
            model: OnceCell::from(model),
            ..Self::default()
        }
    }

    /// Math and code expose the same protected expl3 snapshot. Keep that
    /// semantic index with the snapshot instead of rebuilding it per consumer.
    pub(crate) fn exposed_model(
        &self,
        source: &str,
        config: &Config,
        protected: &[Range<usize>],
    ) -> Rc<crate::source_model::SourceModel> {
        let mut cached = self.exposed_model.borrow_mut();
        if let Some(cached) = &*cached
            && cached.ranges == protected
        {
            return Rc::clone(&cached.model);
        }
        let model = Rc::new(crate::source_model::SourceModel::with_ranges(
            source, config, protected,
        ));
        *cached = Some(ExposedModelCache {
            ranges: protected.to_vec(),
            model: Rc::clone(&model),
        });
        model
    }
}

impl<'a> SourceAnalysis<'a> {
    pub(crate) fn model(&self) -> &crate::source_model::SourceModel {
        if let Some(model) = self.model_ref {
            return model;
        }
        self.memo.model.get_or_init(|| {
            crate::source_model::SourceModel::with_ranges(
                self.source,
                self.config,
                self.protected.unwrap_or(&[]),
            )
        })
    }
    fn has_declaration_tokens(&self) -> bool {
        self.protected.is_none()
            || *self
                .memo
                .declaration_tokens
                .get_or_init(|| crate::syntax::has_analysis_declaration_tokens(self.source))
    }

    pub(crate) fn constructed_commands(
        &self,
        protected: &[Range<usize>],
    ) -> Cow<'_, HashMap<usize, (String, usize)>> {
        // Some public block parsers use broader protection for metadata than
        // for lexing. A constructed command is meaningful only in the ranges
        // used to resolve it; those callers keep a separate owned result.
        if self
            .protected
            .is_some_and(|metadata| std::ptr::eq(metadata, protected) || metadata == protected)
        {
            Cow::Borrowed(self.model().constructed_commands())
        } else {
            Cow::Owned(crate::syntax::constant_constructed_commands(
                self.source,
                self.config,
                protected,
            ))
        }
    }

    pub(crate) fn visual_header_active(&self, name: &str, begin_end: usize) -> bool {
        if self.config.syntax.environment_arguments.contains_key(name) {
            return false;
        }
        let declared_environment = self
            .declared_environments()
            .is_ok_and(|names| names.contains(name));
        let declared_command = self.declared_commands().is_ok_and(|names| {
            names.contains(name) || !names.is_empty() && names.contains(&format!("end{name}"))
        });
        if !declared_environment && !declared_command {
            return true;
        }
        self.memo
            .visual_headers
            .get_or_init(|| {
                self.protected.and_then(|protected| {
                    crate::syntax::visual_header_bindings(self.source, self.config, protected).ok()
                })
            })
            .as_ref()
            .and_then(|headers| headers.get(&begin_end))
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn new(source: &'a str, config: &'a Config, protected: &'a [Range<usize>]) -> Self {
        Self::with_memo(source, config, protected, Rc::default())
    }

    pub(crate) fn for_model(
        source: &'a str,
        config: &'a Config,
        protected: &'a [Range<usize>],
        model: &'a crate::source_model::SourceModel,
    ) -> Self {
        Self {
            source,
            config,
            protected: Some(protected),
            memo: Rc::default(),
            model_ref: Some(model),
        }
    }

    pub(crate) fn with_memo(
        source: &'a str,
        config: &'a Config,
        protected: &'a [Range<usize>],
        memo: Rc<AnalysisMemo>,
    ) -> Self {
        Self {
            source,
            config,
            protected: Some(protected),
            memo,
            model_ref: None,
        }
    }

    pub(crate) fn implicit_list(&self, name: &str) -> bool {
        !self.config.syntax.environment_arguments.contains_key(name)
            && !self
                .declared_environments()
                .is_ok_and(|names| names.contains(name))
    }

    pub(crate) fn box_arguments(&self, name: &str) -> Option<Cow<'_, EnvironmentArguments>> {
        let ordinary = self.memo.arguments.get_or_init(|| {
            let Some(protected) = self.protected else {
                return HashMap::new();
            };
            crate::syntax::environment_argument_map_with_definitions(
                self.source,
                self.config,
                protected,
                &self.definitions(),
            )
        });
        if let Some(arguments) = ordinary.get(name) {
            return Some(Cow::Borrowed(arguments));
        }
        self.memo
            .boxes
            .get_or_init(|| {
                if !self.has_declaration_tokens() {
                    return HashMap::new();
                }
                let Some(protected) = self.protected else {
                    return HashMap::new();
                };
                crate::package_environments::source_box_argument_map(
                    self.source,
                    self.config,
                    protected,
                )
            })
            .get(name)
            .map(Cow::Borrowed)
            .or_else(|| {
                crate::packages::header_signature(name)
                    .filter(|signature| crate::syntax::xparse::extended(signature))
                    .filter(|_| {
                        !self
                            .declared_environments()
                            .is_ok_and(|names| names.contains(name))
                    })
                    .map(|signature| {
                        Cow::Owned(EnvironmentArguments {
                            signature: Some(signature.to_owned()),
                            ..Default::default()
                        })
                    })
            })
    }

    pub(crate) fn box_arguments_at(
        &self,
        name: &str,
        at: usize,
    ) -> Option<Cow<'_, EnvironmentArguments>> {
        if let Some(arguments) = self.model().environment_arguments_after(at) {
            return Some(Cow::Borrowed(arguments));
        }
        // A later declaration cannot supply an earlier call's header, and a
        // local declaration cannot leak across its enclosing scope.
        if self.model().has_environment_declaration(name)
            || self
                .config
                .syntax
                .inferred_environment_arguments
                .contains_key(name)
        {
            return None;
        }
        if let Some(signature) =
            crate::packages::context_header_signature(name, &self.model().invocation_context_at(at))
        {
            return Some(Cow::Owned(EnvironmentArguments {
                signature: Some(signature.into()),
                ..Default::default()
            }));
        }
        self.box_arguments(name)
    }

    fn definitions(&self) -> Cow<'_, [crate::syntax::MacroDefinition]> {
        let analyze = || {
            if !self.has_declaration_tokens() {
                return Vec::new();
            }
            crate::syntax::macro_definitions_from_model(self.source, self.model())
        };
        Cow::Borrowed(self.memo.definitions.get_or_init(analyze))
    }

    pub(crate) fn declared_commands(&self) -> Result<Cow<'_, HashSet<String>>, crate::FormatError> {
        let Some(protected) = self.protected else {
            return crate::syntax::declared_command_names(self.source, self.config).map(Cow::Owned);
        };
        let analyze = || {
            if !self.has_declaration_tokens() {
                return HashSet::new();
            }
            crate::syntax::declared_command_names_with_ranges(
                self.source,
                protected,
                &self.definitions(),
            )
        };
        Ok(Cow::Borrowed(self.memo.commands.get_or_init(analyze)))
    }

    pub(crate) fn declared_environments(
        &self,
    ) -> Result<Cow<'_, HashSet<String>>, crate::FormatError> {
        if self.protected.is_none() {
            return crate::syntax::declared_environment_names(self.source, self.config)
                .map(Cow::Owned);
        }
        let analyze =
            || crate::syntax::declared_environment_names_with_definitions(&self.definitions());
        Ok(Cow::Borrowed(self.memo.environments.get_or_init(analyze)))
    }

    pub(crate) fn inferred_lists(
        &self,
        compiled: &crate::syntax::CompiledSyntax,
    ) -> Cow<'_, [String]> {
        if !self.has_declaration_tokens() {
            return Cow::Borrowed(&[]);
        }
        if ![
            "newenvironment",
            "renewenvironment",
            "DocumentEnvironment",
            "newlist",
        ]
        .iter()
        .any(|name| self.source.contains(name))
        {
            return Cow::Borrowed(&[]);
        }
        let analyze = || {
            if self.protected.is_none() {
                crate::syntax::infer_list_environments_with_config(
                    self.source,
                    &self.config.syntax.list_environments,
                    self.config,
                )
            } else {
                crate::syntax::infer_list_environments_with_definitions(
                    self.source,
                    &self.config.syntax.list_environments,
                    self.config,
                    &self.definitions(),
                    compiled,
                )
            }
        };
        Cow::Borrowed(self.memo.lists.get_or_init(analyze))
    }
}
