pub mod events;
pub mod rpc;
pub mod service;
pub mod types;

pub use service::StreamingService;
pub use types::*;

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::AppResult;

#[async_trait]
pub trait StreamingProvider: Send + Sync {
    async fn start_session(
        &self,
        params: StreamingStartSessionParams,
    ) -> AppResult<StreamingStartSessionResult>;
    async fn get_session_progress(
        &self,
        params: StreamingGetSessionProgressParams,
    ) -> AppResult<Option<StreamingSessionProgressSnapshot>>;
    async fn close_session(
        &self,
        params: StreamingCloseSessionParams,
    ) -> AppResult<StreamingCloseSessionResult>;
    async fn send_keepalive(&self, session_id: String) -> AppResult<bool>;
    async fn exchange_offer(
        &self,
        params: StreamingExchangeOfferParams,
    ) -> AppResult<StreamingExchangeOfferResult>;
    async fn submit_ice(
        &self,
        params: StreamingSubmitIceParams,
    ) -> AppResult<StreamingSubmitIceResult>;
    async fn poll_ice(&self, params: StreamingPollIceParams) -> AppResult<StreamingPollIceResult>;
    async fn list_active_sessions(
        &self,
        params: StreamingListActiveSessionsParams,
    ) -> AppResult<StreamingListActiveSessionsResult>;
    async fn decide_recovery(
        &self,
        params: StreamingDecideRecoveryParams,
    ) -> AppResult<StreamingDecideRecoveryResult>;
    async fn shutdown(&self);
}

pub type StreamingServiceRef = Arc<dyn StreamingProvider>;
