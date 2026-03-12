use serde::{Deserialize, Serialize};

use crate::policy::input::InputPlan;
use crate::policy::negotiation::NegotiationPlan;
use crate::policy::render::RenderPlan;
use crate::policy::runtime::RuntimePlan;
use crate::policy::session::SessionPlan;

/// Plan 是“最终决策”，给 runtime/adapter 消费时不应再出现 Auto。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub session: SessionPlan,
    pub negotiation: NegotiationPlan,
    pub input: InputPlan,
    pub runtime: RuntimePlan,
    pub render: RenderPlan,
}
