use crate::transport::rtc::sdp::policy::apply_offer_policy_contract;
use crate::transport::rtc::sdp::types::RtcSdpContext;

pub(crate) fn adapt_local_offer(raw_offer_sdp: &str, context: &RtcSdpContext) -> String {
    // 沿用旧 transport 的成熟 offer patch 规则，保证新 rtc 栈协商内容与现网策略一致。
    let patched_offer = apply_offer_policy_contract(
        raw_offer_sdp,
        &context.negotiation,
        context.session_target_type.as_ref(),
    );
    let target = context
        .session_target_type
        .as_ref()
        .map(|target| format!("{target:?}"))
        .unwrap_or_else(|| "Unknown".to_string());
    format!(
        "{}a=x-xbx-phase1-rtc:true\r\na=x-xbx-target-type:{}\r\na=x-xbx-offer-profile:{}\r\n",
        patched_offer, target, context.negotiation.offer_profile
    )
}

#[cfg(test)]
mod tests {
    use super::adapt_local_offer;
    use crate::api::runtime::XbxEngineNegotiationRuntimeConfig;
    use crate::transport::rtc::sdp::types::RtcSdpContext;
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    #[test]
    fn local_offer_contains_phase1_tags() {
        let context = RtcSdpContext {
            negotiation: XbxEngineNegotiationRuntimeConfig::default(),
            session_target_type: Some(XbxEngineTargetTypeDto::Home),
        };
        let output = adapt_local_offer("v=0\r\n", &context);
        assert!(output.contains("x-xbx-phase1-rtc:true"));
    }
}
