pub mod api;
pub mod diagnostics;
mod media;
mod platform;
pub mod session;
mod transport;

pub use api::*;
pub use diagnostics::*;
pub use platform::*;
pub use session::*;
// 语义化 facade：runtime_session 表示 engine/runtime 级会话子域。
pub use session as runtime_session;
// 语义化 facade：media_recovery 表示 RTC 媒体链内部恢复子域。
pub use transport::backend::*;
pub use transport::rtc::recovery as media_recovery;

/// 与历史 `crate::runtime_stats_sink::RuntimeStatsSink` 导入路径兼容；实现位于 `diagnostics::sink`。
pub(crate) mod runtime_stats_sink {
    pub(crate) use crate::diagnostics::sink::RuntimeStatsSink;
}
