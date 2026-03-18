//! webrtc 媒体链内部恢复子域。
//!
//! 这里承载四层恢复架构：
//! - signal
//! - diagnosis
//! - policy/coordinator
//! - executor
//!
//! 它只处理媒体链内部的 keyframe / decoder reset / reconnect candidate 语义，
//! 不负责 engine/runtime 级 session 生命周期兜底。

pub mod escalation;
pub mod recovery_coordinator;
pub mod recovery_diagnosis;
pub mod recovery_executor;
pub mod recovery_signal;
pub mod startup_recovery;
