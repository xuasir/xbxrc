use crate::mods::streaming::ice_normalizer::StreamingIceNormalizer;
use crate::mods::streaming::types::{
    StreamingAnswerPayload, StreamingHttpError, StreamingIceCandidate,
};
use serde_json::{json, Value};
use xbox_webapi::{
    IceCandidate as CrateIceCandidate, SdpAnswer, SignalingApi as CrateSignalingApi, WebApiError,
};

#[derive(Clone)]
pub struct StreamingSignalingApi {
    session_base_path: String,
    signaling_api: CrateSignalingApi,
    ice_normalizer: StreamingIceNormalizer,
}

impl StreamingSignalingApi {
    pub fn new(
        session_base_path: String,
        signaling_api: CrateSignalingApi,
        ice_normalizer: StreamingIceNormalizer,
    ) -> Self {
        Self {
            session_base_path,
            signaling_api,
            ice_normalizer,
        }
    }

    pub async fn send_sdp(&self, session_id: &str, sdp: &str) -> Result<(), StreamingHttpError> {
        self.signaling_api
            .send_sdp(session_id, sdp)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })
    }

    pub async fn send_chat_sdp(
        &self,
        session_id: &str,
        sdp: &str,
    ) -> Result<(), StreamingHttpError> {
        self.signaling_api
            .send_chat_sdp(session_id, sdp)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })
    }

    pub async fn get_sdp_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<StreamingAnswerPayload>, StreamingHttpError> {
        let response = self
            .signaling_api
            .get_sdp_exchange_response(session_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        Ok(response.map(|r| StreamingAnswerPayload {
            sdp: r.sdp,
            message_type: r.message_type,
        }))
    }

    pub async fn get_ice_exchange_response(
        &self,
        session_id: &str,
    ) -> Result<Option<Vec<StreamingIceCandidate>>, StreamingHttpError> {
        let response = self
            .signaling_api
            .get_ice_exchange_response(session_id)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })?;

        Ok(response.map(|candidates: Vec<xbox_webapi::IceCandidate>| {
            candidates
                .into_iter()
                .map(|c| StreamingIceCandidate {
                    candidate: c.candidate,
                    sdp_mid: c.sdp_mid,
                    sdp_m_line_index: c.sdp_m_line_index,
                    username_fragment: c.username_fragment,
                    message_type: c.message_type,
                })
                .collect()
        }))
    }

    pub async fn send_ice(
        &self,
        session_id: &str,
        ice: &[StreamingIceCandidate],
    ) -> Result<(), StreamingHttpError> {
        let candidates = ice
            .iter()
            .map(|c| xbox_webapi::IceCandidate {
                candidate: c.candidate.clone(),
                sdp_mid: c.sdp_mid.clone(),
                sdp_m_line_index: c.sdp_m_line_index,
                username_fragment: c.username_fragment.clone(),
                message_type: c.message_type.clone(),
            })
            .collect::<Vec<_>>();

        self.signaling_api
            .send_ice(session_id, &candidates)
            .await
            .map_err(|e| StreamingHttpError {
                status: None,
                body: None,
                message: e.to_string(),
            })
    }
}
