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
    async fn exchange_offer(
        &self,
        params: StreamingExchangeOfferParams,
    ) -> AppResult<StreamingExchangeOfferResult>;
    async fn exchange_ice(
        &self,
        params: StreamingExchangeIceParams,
    ) -> AppResult<StreamingExchangeIceResult>;
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
