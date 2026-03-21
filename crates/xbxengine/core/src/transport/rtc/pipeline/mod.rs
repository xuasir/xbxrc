//! 视频媒体管线，包含会话循环、调度、恢复驱动和观测上报。

pub(crate) mod observation;
pub(crate) mod recovery_driver;
pub(crate) mod recovery_types;
pub(crate) mod scheduler;
pub(crate) mod session;
pub(crate) mod supervisor;
