//! rtc 媒体链恢复子域。
//!
//! 当前恢复架构：
//! - observation：统一观察层（替代signal + diagnosis）
//! - state_machine：恢复状态机（替代escalation + 部分coordinator）
//! - action_coordinator：动作协调器（简化的coordinator）
//!
//! 以及配套的：
//! - startup：启动阶段特殊恢复逻辑
//! - policy：场景策略（Home/Cloud/Relay）

pub(crate) mod contract;
pub(crate) mod coordinator;
pub(crate) mod displayed_idr_fast_path;
pub mod escalation;
pub(crate) mod escalation_label;
pub(crate) mod keyframe_lifecycle;
pub(crate) mod policy;
pub(crate) mod remote_profile_runtime;
pub(crate) mod runtime_state;
pub mod startup;
pub(crate) mod suppress;

// 新的简化模块
pub(crate) mod action_coordinator;
pub(crate) mod observation;
pub(crate) mod state_coordinator;
pub(crate) mod state_machine;
pub(crate) mod timing;
