use super::UINT16SIZE_HALF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SequenceRange {
    pub start: u16,
    pub end_exclusive: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NackWindowAddOutcome {
    pub seq: u16,
    pub duplicate: bool,
    pub is_oos: bool,
    pub oos_distance_from_end: Option<u16>,
    pub advanced_last_consecutive: bool,
    pub overflow_advanced: bool,
    pub opened_gap: Option<SequenceRange>,
    pub closed_gap: Option<SequenceRange>,
    pub overflow_pruned_range: Option<SequenceRange>,
}

pub(super) struct NackSequenceWindow {
    packets: Vec<u64>,
    size: u16,
    end: u16,
    started: bool,
    last_consecutive: u16,
}

impl NackSequenceWindow {
    pub(super) fn new(log2_size_minus_6: u8) -> Self {
        Self {
            packets: vec![0u64; 1 << log2_size_minus_6],
            size: 1 << (log2_size_minus_6 + 6),
            end: 0,
            started: false,
            last_consecutive: 0,
        }
    }

    // 直接沿用 RTC 侧默认 generator 的环形接收窗口语义，避免我们再发明一套缺包判定。
    pub(super) fn add(&mut self, seq: u16) -> NackWindowAddOutcome {
        let mut outcome = NackWindowAddOutcome {
            seq,
            duplicate: false,
            is_oos: false,
            oos_distance_from_end: None,
            advanced_last_consecutive: false,
            overflow_advanced: false,
            opened_gap: None,
            closed_gap: None,
            overflow_pruned_range: None,
        };
        if !self.started {
            self.set_received(seq);
            self.end = seq;
            self.started = true;
            self.last_consecutive = seq;
            return outcome;
        }

        let prev_last_consecutive = self.last_consecutive;
        let last_consecutive_plus1 = self.last_consecutive.wrapping_add(1);
        let diff = seq.wrapping_sub(self.end);
        if diff == 0 {
            outcome.duplicate = true;
            return outcome;
        } else if diff < UINT16SIZE_HALF {
            let gap_start = self.end.wrapping_add(1);
            if gap_start != seq {
                outcome.opened_gap = Some(SequenceRange {
                    start: gap_start,
                    end_exclusive: seq,
                });
            }
            let mut i = self.end.wrapping_add(1);
            while i != seq {
                self.del_received(i);
                i = i.wrapping_add(1);
            }
            self.end = seq;

            let seq_sub_last_consecutive = seq.wrapping_sub(self.last_consecutive);
            if last_consecutive_plus1 == seq {
                self.last_consecutive = seq;
            } else if seq_sub_last_consecutive > self.size {
                let truncated_until = seq.wrapping_sub(self.size);
                outcome.overflow_advanced = true;
                // truncated_until 是新的 last_consecutive，本身不是被放弃的包，
                // 被放弃的区间是 [last_consecutive+1, truncated_until)（不含 truncated_until）
                outcome.overflow_pruned_range = Some(SequenceRange {
                    start: self.last_consecutive.wrapping_add(1),
                    end_exclusive: truncated_until,
                });
                self.last_consecutive = truncated_until;
                // [truncated_until+1, seq-1] 已被 del_received 清空，fix_last_consecutive
                // 在此处实际是空操作（扫描到第一个位置即停），保留调用以维持与 OOS 分支的对称性。
                self.fix_last_consecutive();
            }
        } else {
            outcome.is_oos = true;
            outcome.oos_distance_from_end = Some(self.end.wrapping_sub(seq));
            if last_consecutive_plus1 == seq {
                self.last_consecutive = seq;
                self.fix_last_consecutive();
            }
        }

        self.set_received(seq);
        if self.last_consecutive != prev_last_consecutive {
            outcome.advanced_last_consecutive = true;
            outcome.closed_gap = Some(SequenceRange {
                start: prev_last_consecutive.wrapping_add(1),
                end_exclusive: self.last_consecutive.wrapping_add(1),
            });
        }
        outcome
    }

    pub(super) fn missing_seq_numbers(&self, skip_last_n: u16) -> Vec<u16> {
        let until = self.end.wrapping_sub(skip_last_n);
        let diff = until.wrapping_sub(self.last_consecutive);
        if diff >= UINT16SIZE_HALF {
            return vec![];
        }

        let mut missing = vec![];
        let mut i = self.last_consecutive.wrapping_add(1);
        let until_plus_1 = until.wrapping_add(1);
        while i != until_plus_1 {
            if !self.get_received(i) {
                missing.push(i);
            }
            i = i.wrapping_add(1);
        }
        missing
    }

    pub(super) fn missing_seq_numbers_in_range(&self, start: u16, end_exclusive: u16) -> Vec<u16> {
        let diff = end_exclusive.wrapping_sub(start);
        if diff == 0 || diff >= UINT16SIZE_HALF {
            return vec![];
        }

        let mut missing = Vec::new();
        let mut cursor = start;
        while cursor != end_exclusive {
            if !self.get_received(cursor) {
                missing.push(cursor);
            }
            cursor = cursor.wrapping_add(1);
        }
        missing
    }

    fn set_received(&mut self, seq: u16) {
        let pos = (seq % self.size) as usize;
        self.packets[pos / 64] |= 1u64 << (pos % 64);
    }

    fn del_received(&mut self, seq: u16) {
        let pos = (seq % self.size) as usize;
        self.packets[pos / 64] &= u64::MAX ^ (1u64 << (pos % 64));
    }

    fn get_received(&self, seq: u16) -> bool {
        let pos = (seq % self.size) as usize;
        (self.packets[pos / 64] & (1u64 << (pos % 64))) != 0
    }

    fn fix_last_consecutive(&mut self) {
        let mut i = self.last_consecutive.wrapping_add(1);
        while i != self.end.wrapping_add(1) && self.get_received(i) {
            i = i.wrapping_add(1);
        }
        self.last_consecutive = i.wrapping_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{NackSequenceWindow, SequenceRange};

    #[test]
    fn add_reports_oos_and_gap_closure() {
        let mut window = NackSequenceWindow::new(7);
        let _ = window.add(100);
        let forward = window.add(102);
        assert_eq!(
            forward.opened_gap,
            Some(SequenceRange {
                start: 101,
                end_exclusive: 102
            })
        );
        let oos = window.add(101);
        assert!(oos.is_oos);
        assert!(oos.advanced_last_consecutive);
        assert_eq!(oos.oos_distance_from_end, Some(1));
        assert_eq!(
            oos.closed_gap,
            Some(SequenceRange {
                start: 101,
                end_exclusive: 103
            })
        );
    }

    #[test]
    fn add_reports_overflow_pruned_range() {
        // size = 1 << (0 + 6) = 64
        // add(1): started, last_consecutive=1, end=1
        // add(3): forward gap, end=3, last_consecutive 不推进（3 != 1+1）
        // add(80): seq_sub_last_consecutive = 80-1 = 79 > 64，触发溢出
        //   truncated_until = 80 - 64 = 16
        //   被放弃区间 = [last_consecutive+1, truncated_until) = [2, 16)
        //   end_exclusive = 16（不含 truncated_until=16 本身，它是新的 last_consecutive）
        let mut window = NackSequenceWindow::new(0);
        let _ = window.add(1);
        let _ = window.add(3);
        let outcome = window.add(80);
        assert!(outcome.overflow_advanced);
        assert_eq!(
            outcome.overflow_pruned_range,
            Some(SequenceRange {
                start: 2,
                end_exclusive: 16  // truncated_until 本身不是被放弃的包
            })
        );
    }
}
