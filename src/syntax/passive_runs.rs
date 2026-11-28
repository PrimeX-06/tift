pub(in crate::syntax) struct PassiveRuns(Vec<u32>);

impl PassiveRuns {
    pub(super) fn new(text: &str) -> Self {
        if text.len() > 4 * 1024 * 1024 {
            return Self(Vec::new());
        }
        let bytes = text.as_bytes();
        let mut ends = vec![0; bytes.len() + 1];
        ends[bytes.len()] = bytes.len() as u32;
        let mut line_end = bytes.len();
        for (start, byte) in bytes.iter().copied().enumerate().rev() {
            if byte == b'\n' {
                line_end = start + 1;
            }
            let next = ends[start + 1];
            ends[start] = match byte {
                b'^' | b'!' => u32::MAX,
                b'\\' | b'}' | b'`' => start as u32,
                b'{' if bytes.get(next as usize) == Some(&b'}') => ends[next as usize + 1],
                b'{' => start as u32,
                b'%' => {
                    let comment = text[start + 1..line_end].trim_start();
                    if comment.starts_with('\\')
                        || comment.starts_with("tift:")
                        || comment.starts_with("tex-fmt:")
                        || crate::syntax::is_escaped(text, start)
                    {
                        start as u32
                    } else {
                        ends[line_end]
                    }
                }
                _ => next,
            };
        }
        Self(ends)
    }

    pub(in crate::syntax) fn end(&self, start: usize, limit: usize) -> Option<usize> {
        let end = *self.0.get(start)? as usize;
        (end > start && end <= limit).then_some(end)
    }
}
