use super::UINT16SIZE_HALF;

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

    // 直接沿用 webrtc 默认 generator 的环形接收窗口语义，避免我们再发明一套缺包判定。
    pub(super) fn add(&mut self, seq: u16) {
        if !self.started {
            self.set_received(seq);
            self.end = seq;
            self.started = true;
            self.last_consecutive = seq;
            return;
        }

        let last_consecutive_plus1 = self.last_consecutive.wrapping_add(1);
        let diff = seq.wrapping_sub(self.end);
        if diff == 0 {
            return;
        } else if diff < UINT16SIZE_HALF {
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
                let diff = seq.wrapping_sub(self.size);
                self.last_consecutive = diff;
                self.fix_last_consecutive();
            }
        } else if last_consecutive_plus1 == seq {
            self.last_consecutive = seq;
            self.fix_last_consecutive();
        }

        self.set_received(seq);
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
