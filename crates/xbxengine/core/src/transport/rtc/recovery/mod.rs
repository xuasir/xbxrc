//! rtc 媒体链恢复子域。
//!
//! 四层恢复架构：
//! - signal：观测事实信号
//! - diagnosis：信号 → 恢复原因映射
//! - escalation：burst/cooldown 决策引擎
//! - coordinator：编排完整决策链
//! - executor：由 rtc 顶层 executor 层统一执行
//!
//! 以及配套的：
//! - startup：启动阶段特殊恢复逻辑
//! - policy：场景策略（Home/Cloud/Relay）

pub mod coordinator;
pub(crate) mod decoder_backend_failure;
pub mod diagnosis;
pub mod escalation;
pub(crate) mod hard_stall;
pub(crate) mod nack_outcome;
pub mod policy;
pub(crate) mod repeat_suppression;
pub mod runtime_state;
pub mod signal;
pub mod startup;
