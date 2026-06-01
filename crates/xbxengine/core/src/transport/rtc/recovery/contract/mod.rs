//! L0 恢复合同：按行为域拆分子模块；episode/ledger 在 `session::facts`。

mod completion;
mod decode_sync;
mod display;
mod exit;
mod gap;
mod insert;
mod insert_control;
mod reference_chain;
mod snapshot;
mod sparse_idr;
mod supply;
mod transport_await;

pub(crate) use completion::*;
pub(crate) use decode_sync::*;
pub(crate) use display::*;
pub(crate) use exit::*;
pub(crate) use gap::*;
pub(crate) use insert::*;
pub(crate) use insert_control::*;
pub(crate) use reference_chain::*;
pub(crate) use snapshot::*;
pub(crate) use sparse_idr::*;
pub(crate) use supply::*;
pub(crate) use transport_await::*;

#[cfg(test)]
#[path = "tests.rs"]
mod derive_gap_observation_tests;
