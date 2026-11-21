use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Tex,
    Tikz,
    Style,
    Class,
    Bibtex,
    Dtx,
    Ins,
}

impl FileKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if ["tex", "ltx", "drv"]
            .iter()
            .any(|suffix| extension.eq_ignore_ascii_case(suffix))
        {
            Some(Self::Tex)
        } else if extension.eq_ignore_ascii_case("tikz") {
            Some(Self::Tikz)
        } else if ["sty", "def", "cfg", "fd", "bbx", "cbx", "lbx", "dbx"]
            .iter()
            .any(|suffix| extension.eq_ignore_ascii_case(suffix))
        {
            Some(Self::Style)
        } else if ["cls", "clo"]
            .iter()
            .any(|suffix| extension.eq_ignore_ascii_case(suffix))
        {
            Some(Self::Class)
        } else if extension.eq_ignore_ascii_case("bib") {
            Some(Self::Bibtex)
        } else if ["dtx", "fdd"]
            .iter()
            .any(|suffix| extension.eq_ignore_ascii_case(suffix))
        {
            Some(Self::Dtx)
        } else if extension.eq_ignore_ascii_case("ins") {
            Some(Self::Ins)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub cache: CacheConfig,
    pub indent: IndentConfig,
    pub alignment: crate::alignment::AlignmentConfig,
    pub bibtex: crate::bibtex::BibtexConfig,
    pub code: crate::code_layout::CodeConfig,
    pub math: crate::math_layout::MathConfig,
    pub trees: crate::tree_layout::TreeConfig,
    pub arguments: crate::argument_layout::ArgumentLayoutConfig,
    pub wrap: crate::wrap::WrapConfig,
    pub sentences: crate::wrap::SentenceConfig,
    pub whitespace: WhitespaceConfig,
    pub breaks: BreakConfig,
    pub syntax: SyntaxConfig,
    pub protect: ProtectConfig,
    pub replacements: Vec<crate::replacements::ReplacementConfig>,
    pub files: FilesConfig,
    pub extend: Option<String>,
    pub overrides: Vec<toml::Table>,
    pub replacement_only: bool,
}

/// Persistent CLI results reused by later formatter invocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct CacheConfig {
    pub enabled: bool,
    /// Override the persistent cache directory; the CLI supplies a platform default.
    pub directory: Option<PathBuf>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

#[derive(Debug)]
pub struct CompiledConfig {
    pub config: Config,
    pub(crate) replacements: Arc<Vec<crate::replacements::CompiledReplacement>>,
    pub(crate) alignment: Arc<crate::alignment::CompiledAlignment>,
    pub(crate) text: Arc<crate::wrap::CompiledText>,
    pub(crate) syntax: Arc<crate::syntax::CompiledSyntax>,
}

impl CompiledConfig {
    /// Share the validated syntax with the CLI's project dependency scanner.
    #[doc(hidden)]
    pub fn compiled_syntax(&self) -> &crate::syntax::CompiledSyntax {
        &self.syntax
    }

    pub fn new(config: Config) -> Result<Self, ConfigError> {
        if config.files.threads == Some(0) {
            return Err(ConfigError("files.threads must be positive".into()));
        }
        if config.indent.width == 0 || config.indent.width > 256 {
            return Err(ConfigError("indent.width must be between 1 and 256".into()));
        }
        for (name, rule) in config
            .indent
            .rules
            .iter()
            .chain(config.indent.categories.iter())
        {
            if rule
                .indent
                .as_ref()
                .is_some_and(|s| !s.chars().all(|c| c == ' ' || c == '\t'))
            {
                return Err(ConfigError(format!(
                    "indent rule {name:?} must contain only spaces or tabs"
                )));
            }
            if rule.width.is_some_and(|w| w > 256) {
                return Err(ConfigError(format!(
                    "indent rule {name:?} width exceeds 256"
                )));
            }
        }
        validate_environment_arguments(&config)?;
        crate::bibtex::validate(&config.bibtex).map_err(ConfigError)?;
        crate::argument_layout::validate(&config.arguments).map_err(ConfigError)?;
        crate::math_layout::validate(&config.math).map_err(ConfigError)?;
        crate::tree_layout::validate(&config.trees).map_err(ConfigError)?;
        Ok(Self {
            replacements: Arc::new(
                crate::replacements::compile(&config.replacements).map_err(ConfigError)?,
            ),
            alignment: Arc::new(crate::alignment::compile(&config.alignment).map_err(ConfigError)?),
            text: Arc::new(
                crate::wrap::compile(&config.wrap, &config.sentences).map_err(ConfigError)?,
            ),
            syntax: Arc::new(crate::syntax::compile(&config.syntax).map_err(ConfigError)?),
            config,
        })
    }

    /// Reuse unchanged compiled rules when project declarations only change syntax.
    pub fn with_declaration_config(&self, config: Config) -> Result<Self, ConfigError> {
        let mut before =
            toml::Value::try_from(&self.config).map_err(|e| ConfigError(e.to_string()))?;
        let mut after = toml::Value::try_from(&config).map_err(|e| ConfigError(e.to_string()))?;
        for value in [&mut before, &mut after] {
            let table = value
                .as_table_mut()
                .expect("serialized configuration table");
            table.remove("syntax");
            table.remove("protect");
        }
        if before != after {
            return Self::new(config);
        }
        self.with_project_syntax(config.syntax, config.protect)
    }

    /// Apply inferred syntax while retaining all of this configuration's
    /// validated formatting rules. Taking only the changed fields avoids
    /// serializing unrelated settings during project preparation.
    pub fn with_project_syntax(
        &self,
        syntax_config: SyntaxConfig,
        protect: ProtectConfig,
    ) -> Result<Self, ConfigError> {
        let mut config = self.config.clone();
        config.syntax = syntax_config;
        config.protect = protect;
        // Syntax inferred from literal declarations uses the same validation as
        // user configuration. Other validated settings are unchanged.
        validate_environment_arguments(&config)?;
        let syntax = if self.config.syntax == config.syntax {
            Arc::clone(&self.syntax)
        } else {
            Arc::new(crate::syntax::compile(&config.syntax).map_err(ConfigError)?)
        };
        Ok(Self {
            config,
            replacements: Arc::clone(&self.replacements),
            alignment: Arc::clone(&self.alignment),
            text: Arc::clone(&self.text),
            syntax,
        })
    }
}

fn validate_environment_arguments(config: &Config) -> Result<(), ConfigError> {
    for (name, alias) in &config.syntax.environment_aliases {
        let spelling = format!("\\{name}");
        if name.is_empty()
            || crate::syntax::command_at(&spelling, 0).is_none_or(|(_, end)| end != spelling.len())
            || alias.environment.is_empty()
            || alias
                .environment
                .chars()
                .any(|c| c.is_whitespace() || "\\{}%#".contains(c))
        {
            return Err(ConfigError(format!("invalid environment alias {name:?}")));
        }
    }
    for (name, rule) in &config.syntax.commands {
        if let Some(signature) = &rule.signature {
            if rule.arguments.is_some() || rule.optional_arguments {
                return Err(ConfigError(format!(
                    "command signature for {name:?} cannot be combined with arguments or optional-arguments"
                )));
            }
            if crate::syntax::xparse::Signature::parse(signature).is_none() {
                return Err(ConfigError(format!(
                    "invalid xparse command signature for {name:?}"
                )));
            }
        }
    }
    for (name, rule) in &config.syntax.environment_arguments {
        if rule.arguments > 256 {
            return Err(ConfigError(format!(
                "environment argument count for {name:?} exceeds 256"
            )));
        }
        if let Some(signature) = &rule.signature {
            if rule.arguments != 0 || rule.optional_arguments {
                return Err(ConfigError(format!(
                    "environment signature for {name:?} cannot be combined with arguments or optional-arguments"
                )));
            }
            let valid = if crate::syntax::xparse::extended(signature) {
                crate::syntax::xparse::Signature::parse(signature).is_some()
            } else {
                signature.len() <= 256
            };
            if !valid {
                return Err(ConfigError(format!(
                    "environment signature for {name:?} must use a valid xparse specification or at most 256 slots from m, o, O, p, g, l and d"
                )));
            }
        }
    }
    Ok(())
}

impl Config {
    /// Return an editable TOML configuration containing the stable built-in
    /// settings. Automatically maintained lists stay omitted so later package
    /// support additions continue to apply to generated configurations.
    pub fn default_toml() -> Result<String, ConfigError> {
        let mut value = toml::Value::try_from(Self::default())
            .map_err(|error| ConfigError(error.to_string()))?;
        let root = value
            .as_table_mut()
            .expect("serialized configuration table");
        for (section, field) in [
            ("alignment", "environments"),
            ("syntax", "list-environments"),
            ("protect", "environments"),
            ("arguments", "commands"),
            ("protect", "commands"),
            ("protect", "file-contents"),
            ("protect", "no-indent-blocks"),
            ("sentences", "abbreviations"),
        ] {
            if let Some(table) = root.get_mut(section).and_then(toml::Value::as_table_mut) {
                table.remove(field);
            }
        }
        for key in ["overrides", "replacements"] {
            root.remove(key);
        }
        omit_empty_config_tables(&mut value);
        let root = value
            .as_table_mut()
            .expect("serialized configuration table");
        let mut output = String::from(
            "# tift configuration. Unspecified settings use built-in defaults.\n# Built-in command, environment and abbreviation lists stay dynamic.\n# Explicit arrays replace their inherited lists.\n\n",
        );
        for key in ["extend", "replacement-only"] {
            if let Some(value) = root.remove(key) {
                append_config_section(&mut output, key, value)?;
            }
        }
        for (name, description) in [
            ("indent", "Indentation"),
            ("whitespace", "Whitespace and file endings"),
            ("breaks", "Structural line breaks"),
            ("wrap", "Paragraph wrapping"),
            ("sentences", "Sentence boundaries"),
            ("arguments", "Command argument layout"),
            ("alignment", "Table and equation alignment"),
            ("math", "Mathematical layout"),
            ("trees", "Bracket tree layout"),
            ("code", "Package source and algorithm layout"),
            ("bibtex", "Bibliography layout and optional data cleanup"),
            ("syntax", "Source recognition and custom grammar"),
            ("protect", "Literal source and conservative parsing"),
            ("files", "Project file selection and workers"),
            ("cache", "Persistent formatter cache"),
        ] {
            if let Some(value) = root.remove(name) {
                output.push_str(&format!("# {description}\n"));
                append_config_section(&mut output, name, value)?;
            }
        }
        for (name, value) in std::mem::take(root) {
            append_config_section(&mut output, &name, value)?;
        }
        Ok(output)
    }

    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let value: toml::Value = toml::from_str(source).map_err(|e| ConfigError(e.to_string()))?;
        Self::from_value(value)
    }

    pub fn from_value(value: toml::Value) -> Result<Self, ConfigError> {
        let mut defaults =
            toml::Value::try_from(Self::default()).map_err(|e| ConfigError(e.to_string()))?;
        merge_values(&mut defaults, value);
        defaults
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError(e.to_string()))
    }
}

fn omit_empty_config_tables(value: &mut toml::Value) {
    if let Some(table) = value.as_table_mut() {
        for (_, child) in table.iter_mut() {
            omit_empty_config_tables(child);
        }
        table.retain(|_, child| child.as_table().is_none_or(|table| !table.is_empty()));
    }
}

fn append_config_section(
    output: &mut String,
    name: &str,
    value: toml::Value,
) -> Result<(), ConfigError> {
    let section = toml::Table::from_iter([(name.into(), value)]);
    output.push_str(
        &toml::to_string_pretty(&section).map_err(|error| ConfigError(error.to_string()))?,
    );
    output.push('\n');
    Ok(())
}

/// Tables merge recursively; scalars and arrays replace their inherited value.
pub fn merge_values(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                if let Some(current) = base.get_mut(&key) {
                    merge_values(current, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndentStyle {
    #[default]
    Spaces,
    Tabs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct IndentConfig {
    pub style: IndentStyle,
    pub width: usize,
    pub preamble: bool,
    pub maximum: Option<usize>,
    pub rules: BTreeMap<String, IndentRule>,
    pub categories: BTreeMap<String, IndentRule>,
}

impl Default for IndentConfig {
    fn default() -> Self {
        let mut categories = BTreeMap::new();
        for name in ["command", "group"] {
            categories.insert(
                name.into(),
                IndentRule {
                    additional: false,
                    ..Default::default()
                },
            );
        }
        let mut rules = BTreeMap::new();
        rules.insert(
            "document".into(),
            IndentRule {
                additional: false,
                ..Default::default()
            },
        );
        Self {
            style: IndentStyle::Spaces,
            width: 4,
            preamble: false,
            maximum: None,
            rules,
            categories,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct IndentRule {
    pub additional: bool,
    pub indent: Option<String>,
    pub width: Option<usize>,
}
impl Default for IndentRule {
    fn default() -> Self {
        Self {
            additional: true,
            indent: None,
            width: None,
        }
    }
}

impl IndentConfig {
    pub(crate) fn unit_with_spaces<'a>(
        &'a self,
        category: &str,
        name: &str,
        spaces: &'a str,
    ) -> &'a str {
        let rule = self
            .rules
            .get(name)
            .or_else(|| self.categories.get(category));
        if rule.is_some_and(|rule| !rule.additional) {
            return "";
        }
        if let Some(value) = rule.and_then(|rule| rule.indent.as_deref()) {
            return value;
        }
        match self.style {
            IndentStyle::Spaces => {
                &spaces[..rule.and_then(|rule| rule.width).unwrap_or(self.width)]
            }
            IndentStyle::Tabs => "\t",
        }
    }

    pub fn unit(&self, category: &str, name: &str) -> String {
        let rule = self
            .rules
            .get(name)
            .or_else(|| self.categories.get(category));
        if rule.is_some_and(|r| !r.additional) {
            return String::new();
        }
        if let Some(value) = rule.and_then(|r| r.indent.as_ref()) {
            return value.clone();
        }
        match self.style {
            IndentStyle::Spaces => " ".repeat(rule.and_then(|r| r.width).unwrap_or(self.width)),
            IndentStyle::Tabs => "\t".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct WhitespaceConfig {
    pub remove_trailing: bool,
    pub before_processing: bool,
    pub preserve_blank_lines: bool,
    pub max_blank_lines: Option<usize>,
    pub line_ending: LineEnding,
    pub final_newline: FinalNewline,
}
impl Default for WhitespaceConfig {
    fn default() -> Self {
        Self {
            remove_trailing: true,
            before_processing: false,
            preserve_blank_lines: true,
            max_blank_lines: Some(2),
            line_ending: LineEnding::Preserve,
            final_newline: FinalNewline::Preserve,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LineEnding {
    #[default]
    Preserve,
    Lf,
    Crlf,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FinalNewline {
    #[default]
    Preserve,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BreakMode {
    #[default]
    Preserve,
    Join,
    Line,
    CommentLine,
    BlankLine,
    ResetBlankLine,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct BoundaryRules {
    pub begin: BreakMode,
    pub body: BreakMode,
    pub end: BreakMode,
    pub after: BreakMode,
    pub row: BreakMode,
    pub else_begin: BreakMode,
    pub else_after: BreakMode,
    pub or_begin: BreakMode,
    pub or_after: BreakMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct BreakConfig {
    pub enabled: bool,
    pub categories: BTreeMap<String, BoundaryRules>,
    pub rules: BTreeMap<String, BoundaryRules>,
}
impl Default for BreakConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            categories: BTreeMap::new(),
            rules: BTreeMap::new(),
        }
    }
}

pub(crate) fn matrix_environment(name: &str) -> bool {
    matches!(
        name.strip_suffix('*').unwrap_or(name),
        "matrix" | "pmatrix" | "bmatrix" | "Bmatrix" | "vmatrix" | "Vmatrix" | "smallmatrix"
    )
}

impl BreakConfig {
    pub fn rule(&self, category: &str, name: &str) -> Option<&BoundaryRules> {
        if !self.enabled {
            return None;
        }
        self.rules
            .get(name)
            .or_else(|| self.categories.get(category))
            .or_else(|| {
                if category == "loop" {
                    static LOOP: BoundaryRules = BoundaryRules {
                        begin: BreakMode::Line,
                        body: BreakMode::Line,
                        end: BreakMode::Line,
                        after: BreakMode::Line,
                        row: BreakMode::Preserve,
                        else_begin: BreakMode::Preserve,
                        else_after: BreakMode::Preserve,
                        or_begin: BreakMode::Preserve,
                        or_after: BreakMode::Preserve,
                    };
                    Some(&LOOP)
                } else if category == "environment" {
                    static ENVIRONMENT: BoundaryRules = BoundaryRules {
                        begin: BreakMode::Preserve,
                        body: BreakMode::Line,
                        end: BreakMode::Line,
                        after: BreakMode::Preserve,
                        row: BreakMode::Line,
                        else_begin: BreakMode::Preserve,
                        else_after: BreakMode::Preserve,
                        or_begin: BreakMode::Preserve,
                        or_after: BreakMode::Preserve,
                    };
                    static DOCUMENT: BoundaryRules = BoundaryRules {
                        row: BreakMode::Preserve,
                        ..ENVIRONMENT
                    };
                    Some(if name == "document" {
                        &DOCUMENT
                    } else {
                        &ENVIRONMENT
                    })
                } else if category == "item" {
                    static ITEM: BoundaryRules = BoundaryRules {
                        begin: BreakMode::Line,
                        body: BreakMode::Preserve,
                        end: BreakMode::Preserve,
                        after: BreakMode::Preserve,
                        row: BreakMode::Preserve,
                        else_begin: BreakMode::Preserve,
                        else_after: BreakMode::Preserve,
                        or_begin: BreakMode::Preserve,
                        or_after: BreakMode::Preserve,
                    };
                    Some(&ITEM)
                } else {
                    None
                }
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SyntaxConfig {
    pub foreach: bool,
    pub items: Vec<String>,
    pub list_environments: Vec<String>,
    pub infer_list_environments: bool,
    /// Resource suffixes omitted from project input/include declaration imports.
    /// An empty list allows TeX source with any filename extension.
    pub input_ignore_extensions: Option<Vec<String>>,
    pub conditions: Vec<String>,
    pub headings: BTreeMap<String, HeadingRule>,
    pub commands: BTreeMap<String, CommandRule>,
    pub environment_arguments: BTreeMap<String, EnvironmentArguments>,
    /// Literal control sequences that open or close an environment.
    pub environment_aliases: BTreeMap<String, EnvironmentAlias>,
    /// Project declarations retain source precedence; never serialized as user settings.
    #[serde(skip)]
    pub inferred_environment_aliases: BTreeMap<String, EnvironmentAlias>,
    /// Source-order project imports, anchored to loader occurrences across edits.
    #[serde(skip)]
    pub environment_alias_events: Vec<EnvironmentAliasEvent>,
    #[serde(skip)]
    pub inferred_environment_arguments: BTreeMap<String, EnvironmentArguments>,
    #[serde(skip)]
    pub environment_argument_events: Vec<EnvironmentArgumentEvent>,
    #[serde(skip)]
    pub package_load_events: Vec<PackageLoadEvent>,
    #[serde(skip)]
    pub command_import_events: Vec<CommandImportEvent>,
    pub special: Vec<SpecialRule>,
    pub command_pattern: Option<String>,
    pub environment_pattern: Option<String>,
    pub condition_pattern: Option<String>,
    pub item_pattern: Option<String>,
    pub argument_between: Option<String>,
}
impl Default for SyntaxConfig {
    fn default() -> Self {
        let mut headings = BTreeMap::new();
        for (i, name) in [
            "part",
            "chapter",
            "section",
            "subsection",
            "subsubsection",
            "paragraph",
            "subparagraph",
        ]
        .iter()
        .enumerate()
        {
            headings.insert(
                (*name).into(),
                HeadingRule {
                    level: i + 1,
                    indent: false,
                },
            );
        }
        Self {
            foreach: true,
            items: vec!["item".into()],
            list_environments: [
                "itemize",
                "enumerate",
                "description",
                "list",
                "itemize*",
                "enumerate*",
                "description*",
            ]
            .into_iter()
            .chain(crate::packages::default_list_environments())
            .map(String::from)
            .collect(),
            infer_list_environments: true,
            input_ignore_extensions: None,
            conditions: Vec::new(),
            headings,
            commands: BTreeMap::new(),
            environment_arguments: BTreeMap::new(),
            environment_aliases: BTreeMap::new(),
            inferred_environment_aliases: BTreeMap::new(),
            environment_alias_events: Vec::new(),
            inferred_environment_arguments: BTreeMap::new(),
            environment_argument_events: Vec::new(),
            package_load_events: Vec::new(),
            command_import_events: Vec::new(),
            special: Vec::new(),
            command_pattern: None,
            environment_pattern: None,
            condition_pattern: None,
            item_pattern: None,
            argument_between: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct HeadingRule {
    pub level: usize,
    pub indent: bool,
}
impl Default for HeadingRule {
    fn default() -> Self {
        Self {
            level: 1,
            indent: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct CommandRule {
    pub signature: Option<String>,
    pub arguments: Option<usize>,
    pub optional_arguments: bool,
    pub verbatim: bool,
    pub preserve_body: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct EnvironmentArguments {
    pub arguments: usize,
    pub optional_arguments: bool,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct EnvironmentAlias {
    pub environment: String,
    pub begin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct EnvironmentAliasEvent {
    pub at: usize,
    pub loader: Option<(String, usize)>,
    pub aliases: BTreeMap<String, Option<EnvironmentAlias>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct EnvironmentArgumentEvent {
    pub at: usize,
    pub loader: Option<(String, usize)>,
    pub arguments: BTreeMap<String, Option<EnvironmentArguments>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct PackageLoadEvent {
    pub at: usize,
    pub loader: Option<(String, usize)>,
    pub packages: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ImportedCommand {
    pub arguments: Option<EnvironmentArguments>,
    pub literal_arguments: bool,
    pub global: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CommandImportEvent {
    pub at: usize,
    pub loader: Option<(String, usize)>,
    pub commands: BTreeMap<String, ImportedCommand>,
}

impl EnvironmentArguments {
    pub(crate) fn slots(&self) -> impl Iterator<Item = u8> + '_ {
        self.signature
            .as_deref()
            .into_iter()
            .flat_map(str::bytes)
            .chain(
                std::iter::once(b'o')
                    .filter(|_| self.signature.is_none() && self.optional_arguments),
            )
            .chain(std::iter::repeat_n(
                b'm',
                if self.signature.is_none() {
                    self.arguments
                } else {
                    0
                },
            ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SpecialRule {
    pub name: String,
    pub begin: String,
    pub end: String,
    #[serde(deserialize_with = "deserialize_middle")]
    pub middle: Vec<String>,
    pub regex: bool,
    pub nested: bool,
    pub verbatim: bool,
}

fn deserialize_middle<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Middle {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Middle::deserialize(deserializer)? {
        Middle::One(pattern) => vec![pattern],
        Middle::Many(patterns) => patterns,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ProtectConfig {
    pub environments: Vec<String>,
    pub commands: Vec<String>,
    pub no_indent_blocks: Vec<String>,
    pub file_contents: Vec<String>,
    pub expl3: bool,
    pub dynamic_catcodes: bool,
    pub macro_bodies: bool,
    pub format_macro_bodies: bool,
    pub inactive_branches: bool,
}
impl Default for ProtectConfig {
    fn default() -> Self {
        Self {
            environments: [
                "verbatim",
                "verbatim*",
                "Verbatim",
                "Verbatim*",
                "BVerbatim",
                "LVerbatim",
                "lstlisting",
                "minted",
                "comment",
            ]
            .into_iter()
            .chain(crate::packages::verbatim_environments())
            .map(String::from)
            .collect(),
            commands: ["verb", "lstinline", "mintinline", "mint", "tcboxverb"]
                .map(String::from)
                .to_vec(),
            no_indent_blocks: ["noindent", "cmhtest"].map(String::from).to_vec(),
            file_contents: ["filecontents", "filecontents*"].map(String::from).to_vec(),
            expl3: true,
            dynamic_catcodes: true,
            macro_bodies: true,
            format_macro_bodies: true,
            inactive_branches: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct FilesConfig {
    pub exclude: Vec<String>,
    pub threads: Option<usize>,
}
