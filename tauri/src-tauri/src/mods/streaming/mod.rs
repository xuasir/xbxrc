pub mod api_provider;
pub mod fallback_turn_server_provider;
pub mod http_client;
pub mod ice_normalizer;
pub mod rpc;
pub mod service;
pub mod session_api;
pub mod signaling_api;
pub mod types;

pub use service::StreamingService;
pub use types::*;

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::AppResult;

#[async_trait]
pub trait StreamingProvider: Send + Sync {
    async fn get_fallback_turn_server(
        &self,
        target_type: &str,
    ) -> AppResult<Option<StreamingTurnServerConfig>>;
    async fn create_session(
        &self,
        params: StreamingCreateSessionParams,
    ) -> AppResult<StreamingSessionSnapshot>;
    async fn get_session(
        &self,
        params: StreamingGetSessionParams,
    ) -> AppResult<Option<StreamingSessionSnapshot>>;
    async fn close_session(
        &self,
        params: StreamingCloseSessionParams,
    ) -> AppResult<StreamingCloseSessionResult>;
    async fn exchange_offer(
        &self,
        params: StreamingExchangeOfferParams,
    ) -> AppResult<StreamingExchangeOfferResult>;
    async fn exchange_offer_sdp(
        &self,
        session_id: String,
        channel: Option<String>,
        sdp: String,
    ) -> AppResult<String>;
    async fn exchange_ice(
        &self,
        params: StreamingExchangeIceParams,
    ) -> AppResult<StreamingExchangeIceResult>;
    async fn exchange_ice_candidates(
        &self,
        session_id: String,
        candidates: Vec<StreamingIceCandidate>,
    ) -> AppResult<Vec<StreamingIceCandidate>>;
    async fn send_keepalive(
        &self,
        params: StreamingKeepAliveParams,
    ) -> AppResult<StreamingKeepAliveResult>;
    async fn keep_alive_remote_session(&self, session_id: String) -> AppResult<bool>;
    async fn close_remote_session(&self, session_id: String) -> AppResult<bool>;
    async fn list_active_sessions(
        &self,
        params: StreamingListActiveSessionsParams,
    ) -> AppResult<StreamingListActiveSessionsResult>;
    async fn shutdown(&self);
}

pub type StreamingServiceRef = Arc<dyn StreamingProvider>;
