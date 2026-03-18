//! engine/runtime 级 session 子域。
//!
//! 这里负责的是“整场会话”的生命周期、命令和 watchdog 兜底：
//! - runtime state 与 session commands/events
//! - 会话级 reconnect fallback
//! - 不直接参与 webrtc 视频链内部的逐帧恢复
//!
//! 媒体链内部恢复请看 `transport::webrtc::recovery`。

pub mod commands;
pub mod events;
pub mod recovery;
pub mod session;
pub mod state;

pub use commands::*;
pub use events::*;
pub use recovery::*;
pub use session::*;
pub use state::*;
