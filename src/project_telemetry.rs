use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::InputGraph;

macro_rules! project_metrics {
    ($($name:ident: $ty:ty),+ $(,)?) => {
        #[derive(Default)]
        pub(super) struct ProjectMetrics {
            $(pub(super) $name: $ty),+
        }

        impl ProjectMetrics {
            fn accumulate(&mut self, graph: Self) {
                $(self.$name += graph.$name;)+
            }
        }
    };
}

project_metrics! {
    graphs: usize,
    files: usize,
    visits: usize,
    elapsed: Duration,
    source_bytes: usize,
    append_calls: usize,
    appended_bytes: usize,
    declaration_calls: usize,
    declaration_input_bytes: usize,
    declaration_scan_bytes: usize,
    protection_scan_bytes: usize,
    declaration_incremental_calls: usize,
    declaration_full_scans: usize,
    declaration_elapsed: Duration,
    declaration_cache_hits: usize,
    alias_lexed_bytes: usize,
    export_calls: usize,
    export_input_bytes: usize,
    export_lexed_bytes: usize,
    export_protection_input_bytes: usize,
    ahead_cuts: usize,
    ahead_bytes: usize,
    export_ahead_hits: usize,
    loader_ahead_hits: usize,
    export_elapsed: Duration,
    export_cache_hits: usize,
    prefix_calls: usize,
    prefix_preview_calls: usize,
    prefix_input_bytes: usize,
    prefix_elapsed: Duration,
}

impl ProjectMetrics {
    fn report(&self, root: &Path) {
        let elapsed_ms = self.elapsed.as_secs_f64() * 1000.0;
        eprintln!(
            "trace: project-index: root={}, graphs={}, file-visits={}, files={}, wall={elapsed_ms:.3} ms, source-bytes={}, append-calls={}, appended-bytes={}, declaration-calls={}, declaration-input-bytes={}, declaration-scan-bytes={}, protection-scan-bytes={}, declaration-incremental={}, declaration-full-scans={}, declaration-cache-hits={}, declaration-ms={:.3}, alias-index-lexed-bytes={}, export-calls={}, export-input-bytes={}, export-lexed-bytes={}, export-protection-input-bytes={}, export-ahead-cuts={}, export-ahead-protection-bytes={}, export-ahead-hits={}, loader-ahead-hits={}, export-cache-hits={}, export-ms={:.3}, prefix-calls={}, prefix-previews={}, prefix-suffix-bytes={}, prefix-ms={:.3}, index-ms={elapsed_ms:.3}",
            root.display(),
            self.graphs,
            self.visits,
            self.files,
            self.source_bytes,
            self.append_calls,
            self.appended_bytes,
            self.declaration_calls,
            self.declaration_input_bytes,
            self.declaration_scan_bytes,
            self.protection_scan_bytes,
            self.declaration_incremental_calls,
            self.declaration_full_scans,
            self.declaration_cache_hits,
            self.declaration_elapsed.as_secs_f64() * 1000.0,
            self.alias_lexed_bytes,
            self.export_calls,
            self.export_input_bytes,
            self.export_lexed_bytes,
            self.export_protection_input_bytes + self.ahead_bytes,
            self.ahead_cuts,
            self.ahead_bytes,
            self.export_ahead_hits,
            self.loader_ahead_hits,
            self.export_cache_hits,
            self.export_elapsed.as_secs_f64() * 1000.0,
            self.prefix_calls,
            self.prefix_preview_calls,
            self.prefix_input_bytes,
            self.prefix_elapsed.as_secs_f64() * 1000.0,
        );
    }
}

#[derive(Default)]
pub(super) struct ProjectReports(Mutex<BTreeMap<PathBuf, ProjectMetrics>>);

impl ProjectReports {
    pub(super) fn finish(&self, root: &Path, graph: &mut InputGraph) {
        let trace = graph.trace.take().expect("traced graph");
        debug_assert!(trace.stack.is_empty());
        let mut metrics = graph.metrics.take().expect("traced graph metrics");
        metrics.graphs = 1;
        metrics.files = graph.files.len();
        metrics.visits = trace.visits;
        metrics.elapsed = trace.started.elapsed();
        metrics.source_bytes = graph.source.len();
        metrics.alias_lexed_bytes = graph.alias_index.total_lexed_bytes();
        if let Some(run) = &graph.protection_run {
            (metrics.ahead_cuts, metrics.ahead_bytes) = run.scanned();
        }
        self.0
            .lock()
            .unwrap()
            .entry(root.to_owned())
            .or_default()
            .accumulate(metrics);
    }

    pub(super) fn report(&self) {
        for (root, metrics) in std::mem::take(&mut *self.0.lock().unwrap()) {
            metrics.report(&root);
        }
    }
}

struct Visit {
    started: Instant,
    children: Duration,
    source: usize,
    inference: usize,
}

pub(super) struct ProjectTrace {
    started: Instant,
    stack: Vec<Visit>,
    visits: usize,
}

impl ProjectTrace {
    pub(super) fn new() -> Box<Self> {
        Box::new(Self {
            started: Instant::now(),
            stack: Vec::new(),
            visits: 0,
        })
    }
}

impl InputGraph {
    pub(super) fn trace_enter(&mut self) {
        if let Some(trace) = &mut self.trace {
            trace.stack.push(Visit {
                started: Instant::now(),
                children: Duration::ZERO,
                source: self.source.len(),
                inference: self.source.inference().len(),
            });
        }
    }

    pub(super) fn trace_exit(&mut self, path: &Path, file_bytes: usize) {
        if let Some(trace) = &mut self.trace {
            let depth = trace.stack.len();
            let visit = trace.stack.pop().expect("entered project file");
            let inclusive = visit.started.elapsed();
            if let Some(parent) = trace.stack.last_mut() {
                parent.children += inclusive;
            }
            trace.visits += 1;
            eprintln!(
                "trace: project-file: path={}, depth={depth}, file-bytes={file_bytes}, source={:?}, inference={:?}, own-ms={:.3}, inclusive-ms={:.3}",
                path.display(),
                visit.source..self.source.len(),
                visit.inference..self.source.inference().len(),
                inclusive.saturating_sub(visit.children).as_secs_f64() * 1000.0,
                inclusive.as_secs_f64() * 1000.0,
            );
        }
    }
}
