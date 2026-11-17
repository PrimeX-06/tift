//! Source-dependent environment metadata scoped to one immutable text snapshot.

use std::borrow::Cow;
use std::cell::OnceCell;
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
}

#[derive(Default)]
pub(crate) struct AnalysisMemo {
    declaration_tokens: OnceCell<bool>,
    boxes: OnceCell<HashMap<String, EnvironmentArguments>>,
    arguments: OnceCell<HashMap<String, EnvironmentArguments>>,
    definitions: OnceCell<Vec<crate::syntax::MacroDefinition>>,
    commands: OnceCell<HashSet<String>>,
    command_arguments: OnceCell<HashMap<String, EnvironmentArguments>>,
    environments: OnceCell<HashSet<String>>,
    visual_headers: OnceCell<Option<HashMap<usize, bool>>>,
    constructed_commands: OnceCell<HashMap<usize, (String, usize)>>,
    lists: OnceCell<Vec<String>>,
}

impl<'a> SourceAnalysis<'a> {
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
            Cow::Borrowed(self.memo.constructed_commands.get_or_init(|| {
                crate::syntax::constant_constructed_commands(self.source, self.config, protected)
            }))
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
    }

    fn definitions(&self) -> Cow<'_, [crate::syntax::MacroDefinition]> {
        let analyze = || {
            if !self.has_declaration_tokens() {
                return Vec::new();
            }
            self.protected.map_or_else(Vec::new, |protected| {
                crate::syntax::macro_definitions_with_ranges(self.source, protected)
            })
        };
        Cow::Borrowed(self.memo.definitions.get_or_init(analyze))
    }

    pub(crate) fn command_arguments(&self, name: &str) -> Option<Cow<'_, EnvironmentArguments>> {
        self.memo
            .command_arguments
            .get_or_init(|| {
                let Some(protected) = self.protected else {
                    return HashMap::new();
                };
                crate::syntax::command_argument_map_with_definitions(
                    self.source,
                    self.config,
                    protected,
                    &self.definitions(),
                )
            })
            .get(name)
            .map(Cow::Borrowed)
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


