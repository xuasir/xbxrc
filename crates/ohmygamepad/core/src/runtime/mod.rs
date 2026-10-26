use std::{sync::OnceLock, time::Instant};

mod engine;
mod runner;
mod selector;

pub use engine::*;
pub use runner::*;
pub use selector::*;

static MONOTONIC_ORIGIN: OnceLock<Instant> = OnceLock::new();

/// 进程级单调时钟起点；runtime 与采样层共用同一基准，避免 UNIX 时间与运行时时钟混用。
pub(crate) fn monotonic_origin() -> Instant {
    *MONOTONIC_ORIGIN.get_or_init(Instant::now)
}

/// 进程启动至今的单调毫秒数。
pub fn monotonic_now_ms() -> u64 {
    monotonic_origin().elapsed().as_millis() as u64
}
