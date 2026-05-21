//! RTC 传输子系统（接收侧 trace 与 `ReceiverState` 裁决已分离；观测写入经 `RuntimeStatsSink`）。

pub mod bwe;
pub mod capability;
pub mod connection;
pub mod events;
pub mod executor;
pub mod facts;
pub mod ingress;
pub mod latency;
pub mod pipeline;
pub mod policy;
pub mod projection;
pub mod protocol;
pub mod receive;
pub mod recovery;
pub mod sdp;
pub mod session;
pub mod stack;
pub mod stats;
pub mod stream;
