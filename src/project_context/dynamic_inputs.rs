use super::{Config, ImportSource, syntax};
use rayon::prelude::*;
use std::ops::Range;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct DynamicInputProtection {
    batch: Option<Batch>,
}

struct Batch {
    tape: String,
    options: Arc<Config>,
    cuts: Vec<(usize, Option<Vec<Range<usize>>>)>,
}

impl DynamicInputProtection {
    pub(super) fn ranges(
        &mut self,
        source: &str,
        options: &Arc<Config>,
        file: &ImportSource,
        index: usize,
        disabled_previous: bool,
    ) -> Option<Vec<Range<usize>>> {
        if let Some(batch) = &mut self.batch
            && let Ok(cut) = batch.cuts.binary_search_by_key(&source.len(), |cut| cut.0)
            && batch.tape.get(..source.len()) == Some(source)
            && (Arc::ptr_eq(&batch.options, options)
                || batch.options.syntax == options.syntax
                    && batch.options.protect == options.protect)
            && let Some(ranges) = batch.cuts[cut].1.take()
        {
            return Some(ranges);
        }
        self.batch = None;
        if disabled_previous
            && rayon::current_thread_index().is_some()
            && rayon::current_num_threads() > 1
            && let Some(mut batch) = Batch::scan(source, options, file, index)
        {
            let ranges = batch.cuts[0].1.take();
            self.batch = Some(batch);
            return ranges;
        }
        syntax::protected_ranges(source, options).ok()
    }
}

impl Batch {
    fn scan(
        source: &str,
        options: &Arc<Config>,
        file: &ImportSource,
        index: usize,
    ) -> Option<Self> {
        if !(super::LARGE_PROJECT_PREFIX..=1024 * 1024).contains(&source.len()) {
            return None;
        }
        let mut tail = String::new();
        let mut ends = vec![source.len()];
        let candidates = file.candidates.get(index..)?;
        for pair in candidates.windows(2).take(7) {
            let current = &pair[0];
            let next = &pair[1];
            if !current.name.contains('\\') || !next.name.contains('\\') {
                break;
            }
            let separator = file.text.get(current.span.end..next.span.start)?;
            if !separator.bytes().all(|byte| byte.is_ascii_whitespace())
                || next.span.start - current.span.start > 4096
            {
                break;
            }
            tail.push_str(&file.text[current.span.clone()]);
            tail.push_str("\n\n");
            tail.push_str(separator);
            ends.push(source.len() + tail.len());
        }
        if ends.len() < 3 {
            return None;
        }
        let compiled = syntax::compile(&options.syntax).ok()?;
        let mut tape = String::with_capacity(source.len() + tail.len());
        tape.push_str(source);
        tape.push_str(&tail);
        let cuts = ends
            .into_par_iter()
            .map(|end| {
                let ranges = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    syntax::protected_ranges_with_syntax(&tape[..end], options, &compiled)
                }))
                .ok()
                .and_then(Result::ok);
                (end, ranges)
            })
            .collect();
        Some(Self {
            tape,
            options: Arc::clone(options),
            cuts,
        })
    }
}
