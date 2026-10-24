/// Dense board-local identifier; construction checks the source table bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HexId(u32);
impl HexId {
    pub fn checked(index: usize, table_len: usize) -> Option<Self> {
        (index < table_len)
            .then(|| u32::try_from(index).ok().map(Self))
            .flatten()
    }
    pub fn index(self) -> usize {
        self.0 as usize
    }
}
