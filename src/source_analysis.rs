//! Source-dependent environment metadata scoped to one immutable text snapshot.

use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use crate::Config;
use crate::config::EnvironmentArguments;
use crate::source_document::SourceDocument;
use crate::source_model::SourceModel;

pub(crate) struct SourceAnalysis<'a> {
    source: &'a str,
    config: &'a Config,
    protected: &'a [Range<usize>],
    memo: Rc<AnalysisMemo>,
    model_ref: Option<&'a SourceModel>,
}

#[derive(Default)]
pub(crate) struct AnalysisMemo {
    document: OnceCell<SourceDocument>,
    lexical_protection: Option<Vec<Range<usize>>>,
    reader_windows: RefCell<Vec<crate::syntax::ReaderWindow>>,
    model: OnceCell<Rc<SourceModel>>,
    exposed_model: RefCell<Option<Rc<SourceModel>>>,
    declaration_tokens: OnceCell<bool>,
    boxes: OnceCell<HashMap<String, EnvironmentArguments>>,
    arguments: OnceCell<HashMap<String, EnvironmentArguments>>,
    definitions: OnceCell<Vec<crate::syntax::MacroDefinition>>,
    commands: OnceCell<HashSet<String>>,
    environments: OnceCell<HashSet<String>>,
    visual_headers: OnceCell<Option<HashMap<usize, bool>>>,
    lists: OnceCell<Vec<String>>,
}

impl AnalysisMemo {
    pub(crate) fn existing_model(&self) -> Option<&SourceModel> {
        self.model.get().map(Rc::as_ref)
    }

    pub(crate) fn with_document(
        document: SourceDocument,
        lexical_protection: Vec<Range<usize>>,
        reader_windows: Vec<crate::syntax::ReaderWindow>,
    ) -> Self {
        Self {
            document: OnceCell::from(document),
            lexical_protection: Some(lexical_protection),
            reader_windows: RefCell::new(reader_windows),
            ..Self::default()
        }
    }

    pub(crate) fn with_model(model: SourceModel) -> Self {
        let document = model.document().clone();
        Self {
            document: OnceCell::from(document),
            model: OnceCell::from(Rc::new(model)),
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
    ) -> Rc<SourceModel> {
        if let Some(model) = self.model.get()
            && model.lexical_protection() == protected
        {
            return Rc::clone(model);
        }
        let mut cached = self.exposed_model.borrow_mut();
        if let Some(cached) = &*cached
            && cached.lexical_protection() == protected
        {
            return Rc::clone(cached);
        }
        let document = self
            .document
            .get_or_init(|| SourceDocument::new(source))
            .clone();
        let model = Rc::new(SourceModel::with_document(document, config, protected));
        *cached = Some(Rc::clone(&model));
        model
    }
}

impl<'a> SourceAnalysis<'a> {
    pub(crate) fn model(&self) -> &SourceModel {
        if let Some(model) = self.model_ref {
            return model;
        }
        self.memo.model.get_or_init(|| {
            let document = self
                .memo
                .document
                .get_or_init(|| SourceDocument::new(self.source))
                .clone();
            let mut model = SourceModel::with_document(
                document,
                self.config,
                self.memo
                    .lexical_protection
                    .as_deref()
                    .unwrap_or(self.protected),
            );
            model.set_reader_windows(self.memo.reader_windows.take());
            Rc::new(model)
        })
    }
    fn has_declaration_tokens(&self) -> bool {
        *self
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
        if std::ptr::eq(self.protected, protected) || self.protected == protected {
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
        let names = self.declared_commands();
        if !self.declared_environments().contains(name)
            && !names.contains(name)
            && (names.is_empty() || !names.contains(&format!("end{name}")))
        {
            return true;
        }
        self.memo
            .visual_headers
            .get_or_init(|| {
                crate::syntax::visual_header_bindings(self.source, self.config, self.protected).ok()
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
        model: &'a SourceModel,
    ) -> Self {
        Self {
            source,
            config,
            protected,
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
            protected,
            memo,
            model_ref: None,
        }
    }

    pub(crate) fn implicit_list(&self, name: &str) -> bool {
        !self.config.syntax.environment_arguments.contains_key(name)
            && !self.declared_environments().contains(name)
    }

    pub(crate) fn box_arguments(&self, name: &str) -> Option<Cow<'_, EnvironmentArguments>> {
        let ordinary = self.memo.arguments.get_or_init(|| {
            crate::syntax::environment_argument_map_with_definitions(
                self.source,
                self.config,
                self.protected,
                self.definitions(),
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
                crate::package_environments::source_box_argument_map(
                    self.source,
                    self.config,
                    self.protected,
                )
            })
            .get(name)
            .map(Cow::Borrowed)
            .or_else(|| {
                crate::packages::header_signature(name)
                    .filter(|signature| crate::syntax::xparse::extended(signature))
                    .filter(|_| !self.declared_environments().contains(name))
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

    fn definitions(&self) -> &[crate::syntax::MacroDefinition] {
        self.memo.definitions.get_or_init(|| {
            if self.has_declaration_tokens() {
                crate::syntax::macro_definitions_from_model(self.source, self.model())
            } else {
                Vec::new()
            }
        })
    }

    pub(crate) fn declared_commands(&self) -> &HashSet<String> {
        self.memo.commands.get_or_init(|| {
            if self.has_declaration_tokens() {
                crate::syntax::declared_command_names_with_ranges(
                    self.source,
                    self.protected,
                    self.definitions(),
                )
            } else {
                HashSet::new()
            }
        })
    }

    pub(crate) fn declared_environments(&self) -> &HashSet<String> {
        self.memo.environments.get_or_init(|| {
            crate::syntax::declared_environment_names_with_definitions(self.definitions())
        })
    }

    pub(crate) fn inferred_lists(&self, compiled: &crate::syntax::CompiledSyntax) -> &[String] {
        self.memo.lists.get_or_init(|| {
            if !self.has_declaration_tokens()
                || ![
                    "newenvironment",
                    "renewenvironment",
                    "DocumentEnvironment",
                    "newlist",
                ]
                .iter()
                .any(|name| self.source.contains(name))
            {
                return Vec::new();
            }
            crate::syntax::infer_list_environments_with_definitions(
                self.source,
                &self.config.syntax.list_environments,
                self.config,
                self.definitions(),
                compiled,
            )
        })
    }
}
