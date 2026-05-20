//! Session policy 测试拆分：见仓库计划。子模块按主题分文件，共享 harness。

mod bwe_twcc;
// mod display_owner_ledger;
mod harness;
// mod reconnect_lifecycle;
mod recovery_dynamic_timing_contract;
// RFC 2026-05-20：picture recovery 改由 RtcReceiveCore 本地执行；旧 transport-await 集成合同已移除。
// mod recovery_integration;
mod recovery_observation;
mod stale_runtime_domain;

// #[path = "../playback_phase_integration/mod.rs"]
// mod playback_phase_integration;
