//! Source-ordered package and environment context shared by syntax consumers.

use std::collections::{BTreeMap, HashMap};

pub(crate) fn input_package_at(source: &str, end: usize) -> Option<(&'static str, usize)> {
    let cursor = crate::trivia::skip(source, end);
    let input = source[cursor..]
        .split(|ch: char| ch.is_whitespace() || ch == '%')
        .next()?;
    crate::syntax::input_package(input).map(|package| (package, cursor + input.len()))
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ContextIndex {
    packages: HashMap<String, Vec<PackageState>>,
    environments: Vec<(usize, Option<String>)>,
    stack: Vec<String>,
    imports: HashMap<usize, BTreeMap<String, bool>>,
}

#[derive(Debug, Clone)]
struct PackageState {
    at: usize,
    definite: Option<usize>,
}

#[derive(Clone, Copy)]
pub(crate) struct InvocationContext<'a> {
    index: &'a ContextIndex,
    at: usize,
}

impl InvocationContext<'_> {
    pub(crate) fn has_package(&self, name: &str) -> bool {
        self.package_position(name).is_some()
    }

    pub(crate) fn package_position(&self, name: &str) -> Option<usize> {
        let events = self.index.packages.get(name)?;
        let end = events.partition_point(|event| event.at <= self.at);
        end.checked_sub(1).and_then(|index| events[index].definite)
    }

    pub(crate) fn possibly_has_package(&self, name: &str) -> bool {
        self.index
            .packages
            .get(name)
            .is_some_and(|events| events.first().is_some_and(|event| event.at <= self.at))
    }

    pub(crate) fn environment(&self) -> Option<&str> {
        let end = self
            .index
            .environments
            .partition_point(|(at, _)| *at <= self.at);
        end.checked_sub(1)
            .and_then(|index| self.index.environments[index].1.as_deref())
    }
}

impl ContextIndex {
    pub(crate) fn remove_inner_environments(&mut self, body: &std::ops::Range<usize>) {
        self.environments
            .retain(|(at, _)| *at <= body.start || *at >= body.end);
    }

    pub(crate) fn insert_inner_environments(
        &mut self,
        at: usize,
        local: &Self,
        parent: Option<&str>,
    ) {
        let start = self.environments.partition_point(|(pos, _)| *pos <= at);
        self.environments.splice(
            start..start,
            local
                .environments
                .iter()
                .map(|(pos, name)| (at + pos, name.clone().or_else(|| parent.map(str::to_owned)))),
        );
    }

    pub(crate) fn project_positions(&mut self, change: &crate::source_document::SourceChange) {
        for events in self.packages.values_mut() {
            for event in events {
                event.at = change.position(event.at);
                event.definite = event.definite.map(|at| change.position(at));
            }
        }
        for (at, _) in &mut self.environments {
            *at = change.position(*at);
        }
        self.imports = self
            .imports
            .drain()
            .map(|(at, value)| (change.position(at), value))
            .collect();
    }
    pub(crate) fn with_config(
        config: &crate::Config,
        loader_positions: Option<&crate::source_model::LoaderPositions>,
    ) -> Self {
        let mut context = Self::default();
        if config.syntax.package_load_events.is_empty() {
            return context;
        }
        for event in &config.syntax.package_load_events {
            if let Some(loader) = &event.loader {
                if let Some(at) = loader_positions
                    .and_then(|positions| positions.get(loader))
                    .map(|span| span.start)
                {
                    context
                        .imports
                        .entry(at)
                        .or_default()
                        .extend(event.packages.clone());
                }
            } else {
                context.inject(0, &event.packages);
            }
        }
        context
    }

    pub(crate) fn export_packages(&self) -> BTreeMap<String, bool> {
        self.packages
            .iter()
            .filter_map(|(name, events)| {
                events
                    .last()
                    .map(|event| (name.clone(), event.definite.is_some()))
            })
            .collect()
    }

    pub(crate) fn inject(&mut self, at: usize, packages: &BTreeMap<String, bool>) {
        for (name, &certain) in packages {
            self.load(name, at, certain);
        }
    }

    fn load(&mut self, name: &str, at: usize, certain: bool) {
        if name == "tex-live-zh-cn" {
            self.load("tex-live", at, certain);
        }
        let events = self.packages.entry(name.into()).or_default();
        let definite = if certain {
            Some(at)
        } else {
            events.last().and_then(|event| event.definite)
        };
        events.push(PackageState { at, definite });
    }
    pub(crate) fn at(&self, at: usize) -> InvocationContext<'_> {
        InvocationContext { index: self, at }
    }

    pub(crate) fn skip_environment(&mut self, name: &str, end: usize) {
        if self
            .stack
            .last()
            .is_some_and(|environment| environment == name)
        {
            self.stack.pop();
            self.environments.push((end, self.stack.last().cloned()));
        }
    }

    pub(crate) fn observe_environment(&mut self, name: &str, begin: bool, at: usize) {
        if begin {
            self.stack.push(name.into());
            self.environments.push((at, self.stack.last().cloned()));
        } else {
            self.skip_environment(name, at);
        }
    }

    /// Call from the existing semantic token walk, after literal/disabled
    /// contents and declaration bodies have been excluded.
    pub(crate) fn observe(
        &mut self,
        source: &str,
        name: &str,
        start: usize,
        end: usize,
        certain: bool,
    ) {
        if let Some(packages) = self.imports.remove(&start) {
            for (package, loaded) in packages {
                self.load(&package, start, certain && loaded);
            }
        }
        if name == "input"
            && let Some((package, at)) = input_package_at(source, end)
        {
            self.load(package, at, certain);
        }
        if matches!(
            name,
            "usepackage"
                | "RequirePackage"
                | "RequirePackageWithOptions"
                | "documentclass"
                | "LoadClass"
                | "LoadClassWithOptions"
        ) {
            let mut cursor = crate::trivia::skip(source, end);
            let mut options = "";
            if source.as_bytes().get(cursor) == Some(&b'[') {
                let Some(finish) = crate::syntax::balanced_end(source, cursor, b'[', b']') else {
                    return;
                };
                options = &source[cursor + 1..finish - 1];
                cursor = crate::trivia::skip(source, finish);
            }
            let Some(finish) = crate::syntax::balanced_end(source, cursor, b'{', b'}') else {
                return;
            };
            for package in source[cursor + 1..finish - 1].split(',').map(str::trim) {
                if !package.is_empty()
                    && package
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                {
                    self.load(package, finish, certain);
                    if package == "codedescribe" {
                        let mut listing = true;
                        for option in options.split(',') {
                            let (key, value) = option.split_once('=').unwrap_or((option, "true"));
                            if matches!(key.trim(), "nolisting" | "no listing") {
                                listing = value.trim() == "false";
                            }
                        }
                        if listing {
                            self.load("codelisting", finish, certain);
                        }
                    }
                }
            }
        } else if let Some((environment, begin, finish)) =
            crate::syntax::env_after_command(source, name, end)
        {
            if begin {
                self.stack.push(environment.into());
                self.environments.push((finish, self.stack.last().cloned()));
            } else if self.stack.last().is_some_and(|value| value == environment) {
                self.stack.pop();
                self.environments.push((start, self.stack.last().cloned()));
            }
        }
    }
}
