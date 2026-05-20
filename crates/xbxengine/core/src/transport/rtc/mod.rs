//! RTC 传输子系统。接收侧环正在按 RFC 重构，重构完成前允许保留暂未接线的恢复/观测 API。
#![allow(dead_code)]

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
