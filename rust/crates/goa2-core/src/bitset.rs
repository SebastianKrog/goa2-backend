/// Sized bitset: maps may exceed 512 tiles. Out-of-range writes are errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitSet {
    words: Vec<u64>,
    len: usize,
}
impl BitSet {
    pub fn new(len: usize) -> Self {
        Self {
            words: vec![0; len.div_ceil(64)],
            len,
        }
    }
    pub fn contains(&self, index: usize) -> bool {
        index < self.len && self.words[index / 64] & (1 << (index % 64)) != 0
    }
    pub fn insert(&mut self, index: usize) -> bool {
        assert!(index < self.len, "bit index outside board");
        let old = self.contains(index);
        self.words[index / 64] |= 1 << (index % 64);
        !old
    }
    pub fn remove(&mut self, index: usize) {
        assert!(index < self.len, "bit index outside board");
        self.words[index / 64] &= !(1 << (index % 64));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spans_word_boundaries_and_large_maps() {
        let mut bits = BitSet::new(1025);
        for i in [0, 63, 64, 511, 512, 1024] {
            assert!(bits.insert(i));
            assert!(!bits.insert(i));
        }
        assert!(!bits.contains(1025));
        bits.remove(512);
        assert!(!bits.contains(512));
        assert!(bits.contains(511));
    }
}
