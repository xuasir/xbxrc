use serde::{Deserialize, Serialize};

use crate::policy::input::TitleCapabilities;
use crate::policy::runtime::{RuntimeCapabilities, TurnContext};
use crate::policy::session::SessionAccessContext;
use crate::policy::types::{HostAddr, Target};

/// xHome 侧 console 地址来自 `/configuration`，主要用于 ICE 注入。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlayContext {
    pub console_addrs: Vec<HostAddr>,
}

/// 编译 plan 需要的运行事实，不属于用户持久化配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub target: Target,
    pub target_id: String,
    pub session: SessionAccessContext,
    pub input: TitleCapabilities,
    pub runtime: RuntimeCapabilities,
    pub remote_play: RemotePlayContext,
    pub turn: TurnContext,
}
