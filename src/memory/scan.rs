pub(super) struct ChunkScanner<'a> {
    needle: &'a [u8],
    carry: Vec<u8>,
    last_end: Option<u64>,
    matches: Vec<u64>,
}

impl<'a> ChunkScanner<'a> {
    pub(super) fn new(needle: &'a [u8]) -> Self {
        Self {
            needle,
            carry: Vec::new(),
            last_end: None,
            matches: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, address: u64, chunk: &[u8]) {
        if self.needle.is_empty() {
            return;
        }
        if self.last_end != Some(address) {
            self.carry.clear();
        }

        let start = address.saturating_sub(self.carry.len() as u64);
        let mut bytes = std::mem::take(&mut self.carry);
        bytes.extend_from_slice(chunk);
        self.matches.extend(
            bytes
                .windows(self.needle.len())
                .enumerate()
                .filter(|(_, window)| *window == self.needle)
                .map(|(offset, _)| start + offset as u64),
        );
        let overlap = self.needle.len().saturating_sub(1).min(bytes.len());
        self.carry
            .extend_from_slice(&bytes[bytes.len() - overlap..]);
        self.last_end = address.checked_add(chunk.len() as u64);
    }

    pub(super) fn push_read(
        &mut self,
        address: u64,
        bytes: &[u8],
        requested: usize,
        succeeded: bool,
    ) -> bool {
        if succeeded && bytes.len() == requested {
            self.push(address, bytes);
            true
        } else {
            self.unreadable_gap();
            false
        }
    }

    pub(super) fn unreadable_gap(&mut self) {
        self.carry.clear();
        self.last_end = None;
    }

    pub(super) fn finish(mut self) -> Vec<u64> {
        self.matches.sort_unstable();
        self.matches.dedup();
        self.matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_signature_split_across_adjacent_chunks() {
        let mut scanner = ChunkScanner::new(b"ABCD");
        scanner.push(100, b"xxAB");
        scanner.push(104, b"CDyy");

        assert_eq!(scanner.finish(), [102]);
    }

    #[test]
    fn does_not_bridge_an_unreadable_region() {
        let mut scanner = ChunkScanner::new(b"ABCD");
        scanner.push(100, b"xxAB");
        scanner.unreadable_gap();
        scanner.push(104, b"CDyy");

        assert!(scanner.finish().is_empty());
    }

    #[test]
    fn returns_overlapping_matches_without_duplicates() {
        let mut scanner = ChunkScanner::new(b"aaa");
        scanner.push(0x1000, b"aaaaa");

        assert_eq!(scanner.finish(), [0x1000, 0x1001, 0x1002]);
    }

    #[test]
    fn failed_partial_reads_are_gaps_and_cannot_supply_matches() {
        let mut scanner = ChunkScanner::new(b"ABCD");
        scanner.push_read(100, b"ABCD", 8, false);
        scanner.push_read(108, b"tail", 4, true);

        assert!(scanner.finish().is_empty());
    }
}
