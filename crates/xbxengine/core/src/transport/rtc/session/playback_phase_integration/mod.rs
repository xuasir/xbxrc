//! 播放期跨模块集成测试（RFC: `docs/rfcs/2026-04-11-playback-phase-cross-module-integration-tests.md`）。
//! - **合成 fixture**，不读取 `runtime-logs`。
//! - 子模块按 Case ID 分段，避免单文件过大。
//! - 时间戳须与墙钟对齐：`session/facts::build_scheduling_demand_signal` 使用 `now_ms_f64()`。

mod common;
mod edge_01_09;
mod edge_10_18;
mod int_01_06;
mod int_07_12;
mod int_13_18;
