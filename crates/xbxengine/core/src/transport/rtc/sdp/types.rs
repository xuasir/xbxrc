use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::api::runtime::XbxEngineNegotiationRuntimeConfig;

#[derive(Clone, Debug)]
pub(crate) struct RtcSdpContext {
    pub(crate) negotiation: XbxEngineNegotiationRuntimeConfig,
    pub(crate) session_target_type: Option<XbxEngineTargetTypeDto>,
}
