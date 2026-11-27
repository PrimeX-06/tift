use super::{CompiledSyntax, command_at, conditions::canonical_condition_control};
use std::sync::LazyLock;

const MAX_CLOSES: usize = 2048;

#[derive(Clone, Default)]
pub(super) struct ClosingIndex {
    closes: Vec<usize>,
    constructed_end: Option<usize>,
}

impl ClosingIndex {
    pub(super) fn extend(&mut self, source: &str, restart: usize) {
        static MARKERS: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"\\(?:fi|MH_fi:|csname)").expect("conditional closing spellings")
        });
        if self.closes.len() > MAX_CLOSES || self.constructed_end.is_some() {
            return;
        }
        self.closes
            .truncate(self.closes.partition_point(|&at| at < restart));
        for marker in MARKERS.find_iter(&source[restart..]) {
            if marker.as_str() == r"\csname" {
                self.constructed_end = Some(restart + marker.end());
                break;
            }
            let at = restart + marker.start();
            if command_at(source, at).is_some_and(|(name, _)| {
                canonical_condition_control(name.trim_end_matches('*')) == "fi"
            }) {
                self.closes.push(at);
                if self.closes.len() > MAX_CLOSES {
                    break;
                }
            }
        }
    }

    pub(super) fn token_start_limit(&self, source: &str, compiled: &CompiledSyntax) -> usize {
        if source.len() < 32 * 1024
            || !source.ends_with('\n')
            || !compiled.specials.is_empty()
            || self.closes.len() > MAX_CLOSES
            || self.constructed_end.is_some_and(|end| end <= source.len())
        {
            return source.len();
        }
        let count = self.closes.partition_point(|&at| at < source.len());
        self.closes[..count].last().map_or(0, |&at| at + 1)
    }
}
