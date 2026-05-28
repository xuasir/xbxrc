//! L0 恢复合同：按行为域拆分子模块；episode/ledger 在 `session::facts`。

mod decode_sync;
mod display;
mod exit;
mod gap;
mod insert;
mod snapshot;
mod sparse_idr;
mod supply;
mod transport_await;

pub use decode_sync::*;
pub use display::*;
pub use exit::*;
pub use gap::*;
pub use insert::*;
pub use snapshot::*;
pub use sparse_idr::*;
pub use supply::*;
pub use transport_await::*;

#[cfg(test)]
#[path = "tests.rs"]
mod derive_gap_observation_tests;
