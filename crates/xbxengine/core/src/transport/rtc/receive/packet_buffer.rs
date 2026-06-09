use std::collections::BTreeMap;

const UINT16SIZE_HALF: u16 = 1 << 15;

/// seq continuity / reorder window（receiver-local，无 chain debt）。
#[derive(Debug, Default)]
pub struct PacketBuffer {
    highest_sequence: Option<u16>,
    gaps: BTreeMap<u16, ()>,
}

#[derive(Debug, Clone, Default)]
pub struct SequenceObserveOutcome {
    pub sequence: u16,
    pub is_duplicate: bool,
    pub is_reorder: bool,
    pub reorder_distance_from_highest: Option<u16>,
    pub newly_opened_gaps: Vec<u16>,
    pub resolved_pending_nack: bool,
}

impl PacketBuffer {
    pub fn observe_sequence(&mut self, sequence: u16, _now_ms: f64) -> SequenceObserveOutcome {
        let mut outcome = SequenceObserveOutcome {
            sequence,
            ..Default::default()
        };
        let Some(highest) = self.highest_sequence else {
            self.highest_sequence = Some(sequence);
            return outcome;
        };
        if sequence == highest {
            outcome.is_duplicate = true;
            return outcome;
        }
        let delta = sequence.wrapping_sub(highest);
        if delta > 0 && delta < UINT16SIZE_HALF {
            let mut expected = highest.wrapping_add(1);
            while expected != sequence {
                if !self.gaps.contains_key(&expected) {
                    self.gaps.insert(expected, ());
                    outcome.newly_opened_gaps.push(expected);
                }
                expected = expected.wrapping_add(1);
            }
            self.highest_sequence = Some(sequence);
            return outcome;
        }
        outcome.is_reorder = true;
        outcome.reorder_distance_from_highest = Some(highest.wrapping_sub(sequence));
        if !self.gaps.contains_key(&sequence) {
            self.gaps.insert(sequence, ());
            outcome.newly_opened_gaps.push(sequence);
        }
        outcome
    }

    pub fn resolve_sequence(&mut self, sequence: u16) {
        self.gaps.remove(&sequence);
    }

    pub fn clear_gaps(&mut self) {
        self.gaps.clear();
    }

    pub fn all_missing(&self) -> Vec<u16> {
        self.gaps.keys().copied().collect()
    }

    pub fn missing_in_range(&self, start: u16, end_exclusive: u16) -> Vec<u16> {
        let span = end_exclusive.wrapping_sub(start);
        if span == 0 || span >= UINT16SIZE_HALF {
            return Vec::new();
        }
        self.gaps
            .keys()
            .copied()
            .filter(|sequence| {
                let offset = sequence.wrapping_sub(start);
                offset < span
            })
            .collect()
    }

    pub fn has_active_gap(&self) -> bool {
        !self.gaps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_forward_gap() {
        let mut buffer = PacketBuffer::default();
        assert!(buffer
            .observe_sequence(100, 0.0)
            .newly_opened_gaps
            .is_empty());
        let outcome = buffer.observe_sequence(103, 1.0);
        assert_eq!(outcome.newly_opened_gaps, vec![101, 102]);
    }

    #[test]
    fn reorder_marks_distance_from_highest() {
        let mut buffer = PacketBuffer::default();
        let _ = buffer.observe_sequence(100, 0.0);
        let _ = buffer.observe_sequence(102, 1.0);
        let outcome = buffer.observe_sequence(101, 2.0);
        assert!(outcome.is_reorder);
        assert_eq!(outcome.reorder_distance_from_highest, Some(1));
    }

    #[test]
    fn missing_in_range_filters_open_gaps() {
        let mut buffer = PacketBuffer::default();
        let _ = buffer.observe_sequence(100, 0.0);
        let _ = buffer.observe_sequence(104, 1.0);
        assert_eq!(buffer.missing_in_range(100, 105), vec![101, 102, 103]);
    }
}
