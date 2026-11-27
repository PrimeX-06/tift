#[derive(Clone, Debug, Default)]
pub(crate) struct NameFilter([u64; 4]);

impl NameFilter {
    #[inline]
    fn bucket(name: &str) -> (usize, u64) {
        let bytes = name.as_bytes();
        let slot = bytes.first().copied().unwrap_or(0) as usize
            ^ (bytes.last().copied().unwrap_or(0) as usize * 7)
            ^ bytes.len().wrapping_mul(31);
        ((slot >> 6) & 3, 1 << (slot & 63))
    }

    pub(crate) fn insert(&mut self, name: &str) {
        let (bucket, bit) = Self::bucket(name);
        self.0[bucket] |= bit;
    }

    #[inline]
    pub(crate) fn may_contain(&self, name: &str) -> bool {
        let (bucket, bit) = Self::bucket(name);
        self.0[bucket] & bit != 0
    }
}
