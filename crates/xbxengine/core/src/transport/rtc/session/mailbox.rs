use std::collections::VecDeque;

use crate::transport::rtc::facts::TransportFact;

/// 轻量有序邮箱：先用于骨架接入，后续可替换为 actor runtime。
#[derive(Default)]
pub struct SessionMailbox {
    queue: VecDeque<TransportFact>,
}

impl SessionMailbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_fact(&mut self, fact: TransportFact) {
        self.queue.push_back(fact);
    }

    pub fn pop_fact(&mut self) -> Option<TransportFact> {
        self.queue.pop_front()
    }
}
