use serde_json::Value;
use xbox_webapi::{
    IceCandidate as WebApiIceCandidate, SignalingApi as CrateSignalingApi, WebApiError,
};

use crate::policy::Plan;
use crate::session::signaling::ice::{IceCandidate, IcePolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerPayload {
    pub sdp: String,
    pub message_type: Option<String>,
}

/// 基于 xbox-webapi 的 signaling 网关：统一 SDP/ICE 交换与候选归一化。
#[derive(Clone)]
pub struct WebApiSignalingGateway {
    signaling_api: CrateSignalingApi,
    ice_policy: IcePolicy,
}

impl WebApiSignalingGateway {
    /// RFC: 凭证属于执行层，结合 Plan 策略构建 Gateway。
    pub fn from_plan_with_token(
        plan: Plan,
        token: Value,
    ) -> Result<Self, crate::session::SessionFlowError> {
        let target_type = if plan.session.target.is_home() {
            "home"
        } else {
            "cloud"
        };

        let gs_token = token
            .get("data")
            .unwrap_or(&token)
            .get("gsToken")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                crate::session::SessionFlowError::message("gsToken missing in credential")
            })?
            .to_string();

        let session_base_path = format!("{}/v5/sessions/{target_type}", plan.session.base_url);
        let signaling_api = CrateSignalingApi::new(session_base_path, gs_token);

        Ok(Self {
            signaling_api,
            ice_policy: IcePolicy::new(plan.negotiation.prefer_ipv6),
        })
    }

    pub async fn send_sdp(&self, session_id: &str, sdp: &str) -> Result<(), WebApiError> {
        self.signaling_api.send_sdp(session_id, sdp).await
    }

    pub async fn send_chat_sdp(&self, session_id: &str, sdp: &str) -> Result<(), WebApiError> {
        self.signaling_api.send_chat_sdp(session_id, sdp).await
    }

    pub async fn get_sdp_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<AnswerPayload>, WebApiError> {
        let response = self
            .signaling_api
            .get_sdp_exchange_response(session_id)
            .await?;

        Ok(response.map(|item| AnswerPayload {
            sdp: item.sdp,
            message_type: item.message_type,
        }))
    }

    pub async fn get_ice_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<Vec<IceCandidate>>, WebApiError> {
        let response = self
            .signaling_api
            .get_ice_exchange_response(session_id)
            .await?;
        Ok(response.map(|items| self.normalize_ice_candidates(items)))
    }

    pub async fn send_ice(
        &self,
        session_id: &str,
        ice: &[IceCandidate],
    ) -> Result<(), WebApiError> {
        let candidates = ice
            .iter()
            .map(|candidate| WebApiIceCandidate {
                candidate: candidate.candidate.clone(),
                sdp_mid: candidate.sdp_mid.clone(),
                sdp_m_line_index: candidate.sdp_m_line_index,
                username_fragment: candidate.username_fragment.clone(),
                message_type: candidate.message_type.clone(),
            })
            .collect::<Vec<_>>();

        self.signaling_api.send_ice(session_id, &candidates).await
    }

    fn normalize_ice_candidates(&self, candidates: Vec<WebApiIceCandidate>) -> Vec<IceCandidate> {
        let raw = candidates
            .into_iter()
            .map(|candidate| IceCandidate {
                candidate: candidate.candidate,
                sdp_mid: candidate.sdp_mid,
                sdp_m_line_index: candidate.sdp_m_line_index,
                username_fragment: candidate.username_fragment,
                message_type: candidate.message_type,
            })
            .collect::<Vec<_>>();
        self.ice_policy.normalize(&raw)
    }
}
