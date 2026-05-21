//! 视频媒体管线，负责会话循环与观测上报（不承载恢复/BWE主流程）。

pub(crate) mod observation;
pub(crate) mod session;
mod session_loop;
pub(crate) mod supervisor;
