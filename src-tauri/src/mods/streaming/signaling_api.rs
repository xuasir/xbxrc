use crate::mods::streaming::ice_normalizer::StreamingIceNormalizer;
use crate::mods::streaming::types::{StreamingAnswerPayload, StreamingIceCandidate};
use xbox_webapi::{IceCandidate, SignalingApi as CrateSignalingApi, WebApiError};

#[derive(Clone)]
pub struct StreamingSignalingApi {
    signaling_api: CrateSignalingApi,
    ice_normalizer: StreamingIceNormalizer,
}

impl StreamingSignalingApi {
    pub fn new(
        _session_base_path: String,
        signaling_api: CrateSignalingApi,
        ice_normalizer: StreamingIceNormalizer,
    ) -> Self {
        Self {
            signaling_api,
            ice_normalizer,
        }
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
    ) -> Result<Option<StreamingAnswerPayload>, WebApiError> {
        let response = self
            .signaling_api
            .get_sdp_exchange_response(session_id)
            .await?;

        Ok(response.map(|r| StreamingAnswerPayload {
            sdp: r.sdp,
            message_type: r.message_type,
        }))
    }

    pub async fn get_ice_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<Vec<StreamingIceCandidate>>, WebApiError> {
        let response = self
            .signaling_api
            .get_ice_exchange_response(session_id)
            .await?;

        Ok(response.map(|candidates| self.normalize_ice_candidates(candidates)))
    }

    pub async fn send_ice(
        &self,
        session_id: &str,
        ice: &[StreamingIceCandidate],
    ) -> Result<(), WebApiError> {
        let candidates = ice
            .iter()
            .map(|c| IceCandidate {
                candidate: c.candidate.clone(),
                sdp_mid: c.sdp_mid.clone(),
                sdp_m_line_index: c.sdp_m_line_index,
                username_fragment: c.username_fragment.clone(),
                message_type: c.message_type.clone(),
            })
            .collect::<Vec<_>>();

        self.signaling_api.send_ice(session_id, &candidates).await
    }

    fn normalize_ice_candidates(
        &self,
        candidates: Vec<IceCandidate>,
    ) -> Vec<StreamingIceCandidate> {
        let tauri_candidates = candidates
            .into_iter()
            .map(|candidate| StreamingIceCandidate {
                candidate: candidate.candidate,
                sdp_mid: candidate.sdp_mid,
                sdp_m_line_index: candidate.sdp_m_line_index,
                username_fragment: candidate.username_fragment,
                message_type: candidate.message_type,
            })
            .collect::<Vec<_>>();

        self.ice_normalizer.normalize(&tauri_candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::StreamingSignalingApi;
    use crate::mods::streaming::ice_normalizer::StreamingIceNormalizer;
    use xbox_webapi::IceCandidate;

    #[test]
    fn normalize_ice_candidates_filters_and_appends_end_marker() {
        let api = StreamingSignalingApi::new(
            "https://example.com/v5/sessions/cloud".to_string(),
            xbox_webapi::SignalingApi::new(
                "https://example.com/v5/sessions/cloud".to_string(),
                "token".to_string(),
            ),
            StreamingIceNormalizer::new(false),
        );

        let normalized = api.normalize_ice_candidates(vec![
            IceCandidate {
                candidate: "a=candidate:foo 1 UDP 1234 10.0.0.1 9000 typ host".to_string(),
                sdp_mid: Some("1".to_string()),
                sdp_m_line_index: Some(1),
                username_fragment: Some("abc".to_string()),
                message_type: None,
            },
            IceCandidate {
                candidate: "a=end-of-candidates".to_string(),
                sdp_mid: Some("1".to_string()),
                sdp_m_line_index: Some(1),
                username_fragment: None,
                message_type: None,
            },
        ]);

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].sdp_mid.as_deref(), Some("0"));
        assert_eq!(normalized[0].sdp_m_line_index, Some(0));
        assert_eq!(normalized[0].message_type.as_deref(), Some("iceCandidate"));
        assert_eq!(normalized[1].candidate, "a=end-of-candidates");
    }
}
