//! Xbox 公共串流线协议与远端场景合同。
mod remote_profile;
mod stream_data_channel;
pub use remote_profile::*;
pub use stream_data_channel::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionTargetType {
    Home,
    Cloud,
}
