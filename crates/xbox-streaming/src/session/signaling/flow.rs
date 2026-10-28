//! 信令事务：阶段期限、协商串行化、当前资源身份与增量候选提交。
use super::*;

impl<S, P> SessionFlowService<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub async fn exchange_offer(
        &self,
        session_id: &str,
        channel: Option<&str>,
        sdp: &str,
        restart: bool,
    ) -> Result<AnswerPayload, SessionFlowError> {
        let record = self
            .get_session_record(session_id)
            .await
            .ok_or_else(|| missing_session_error(session_id))?;
        record
            .cancelled
            .run(
                "exchangeOffer",
                Some(std::time::Duration::from_millis(
                    record.plan.session.schedule.offer_exchange_timeout_ms,
                )),
                async {
                    let mutation = self
                        .session_mutation(session_id, &record.cancelled)
                        .await
                        .ok_or_else(|| missing_session_error(session_id))?;
                    // SDP 协商共享串行入口；生命周期撤销通过 token 即时打断等待。
                    let _negotiation = mutation.negotiation.lock().await;
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    let api = self.create_signaling_api(&record.plan).await?;
                    let previous = if restart {
                        api.get_sdp_exchange_response(session_id)
                            .await
                            .map_err(map_webapi_error)?
                    } else {
                        None
                    };
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    if channel == Some("chat") {
                        api.send_chat_sdp(session_id, sdp)
                            .await
                            .map_err(map_webapi_error)?;
                    } else {
                        api.send_sdp(session_id, sdp)
                            .await
                            .map_err(map_webapi_error)?;
                    }
                    loop {
                        let answer = api
                            .get_sdp_exchange_response(session_id)
                            .await
                            .map_err(map_webapi_error)?;
                        // await 后先复核当前资源，随后允许 answer 提交。
                        self.ensure_current_session(session_id, &record.cancelled)
                            .await?;
                        if let Some(answer) =
                            filter_stale_offer_response(answer, previous.as_ref(), restart)
                        {
                            return Ok(answer);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(
                            record.plan.session.schedule.offer_poll_interval_ms.max(100),
                        ))
                        .await;
                    }
                },
            )
            .await
    }

    pub async fn submit_ice(
        &self,
        session_id: &str,
        candidates: &[IceCandidate],
        restart: bool,
    ) -> Result<(), SessionFlowError> {
        let record = self
            .get_session_record(session_id)
            .await
            .ok_or_else(|| missing_session_error(session_id))?;
        record
            .cancelled
            .run(
                "submitIce",
                Some(std::time::Duration::from_secs(15)),
                async {
                    let mutation = self
                        .session_mutation(session_id, &record.cancelled)
                        .await
                        .ok_or_else(|| missing_session_error(session_id))?;
                    let _ice = mutation.ice.lock().await;
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    let api = self.create_signaling_api(&record.plan).await?;
                    api.send_ice(session_id, candidates)
                        .await
                        .map_err(map_webapi_error)?;
                    let sessions = self.inner.sessions.read().await;
                    ensure_record_current(&sessions, session_id, &record.cancelled)?;
                    let mut signaling = self.inner.signaling.write().await;
                    let state = signaling.entry(session_id.to_owned()).or_default();
                    if restart {
                        state.restart_baseline_ice = None;
                        state.last_polled_ice = None;
                    }
                    Ok(())
                },
            )
            .await
    }

    pub async fn poll_ice(
        &self,
        session_id: &str,
        restart: bool,
    ) -> Result<Vec<IceCandidate>, SessionFlowError> {
        let record = self
            .get_session_record(session_id)
            .await
            .ok_or_else(|| missing_session_error(session_id))?;
        record
            .cancelled
            .run(
                "pollIce",
                Some(std::time::Duration::from_millis(
                    record.plan.session.schedule.ice_exchange_timeout_ms,
                )),
                async {
                    let mutation = self
                        .session_mutation(session_id, &record.cancelled)
                        .await
                        .ok_or_else(|| missing_session_error(session_id))?;
                    let _ice = mutation.ice.lock().await;
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    let api = self.create_signaling_api(&record.plan).await?;
                    let response = api
                        .get_ice_exchange_response(session_id)
                        .await
                        .map_err(map_webapi_error)?;
                    let response = self
                        .filter_polled_ice_response(
                            session_id,
                            &record.cancelled,
                            response,
                            restart,
                        )
                        .await?;
                    resolve_polled_ice_result(session_id, response, true)
                },
            )
            .await
    }

    async fn ensure_current_session(
        &self,
        session_id: &str,
        cancelled: &SessionCancelToken,
    ) -> Result<(), SessionFlowError> {
        ensure_record_current(&*self.inner.sessions.read().await, session_id, cancelled)
    }

    async fn filter_polled_ice_response(
        &self,
        session_id: &str,
        cancelled: &SessionCancelToken,
        response: Option<Vec<IceCandidate>>,
        _restart: bool,
    ) -> Result<Option<Vec<IceCandidate>>, SessionFlowError> {
        let sessions = self.inner.sessions.read().await;
        ensure_record_current(&sessions, session_id, cancelled)?;
        let mut signaling = self.inner.signaling.write().await;
        let state = signaling.entry(session_id.to_owned()).or_default();
        if response.as_deref() == state.last_polled_ice.as_deref() {
            return Ok(None);
        }
        if let Some(candidates) = response.as_ref() {
            state.last_polled_ice = Some(candidates.clone());
        }
        Ok(response)
    }
}

fn ensure_record_current<S: Clone>(
    sessions: &SessionRuntimeStore<S>,
    session_id: &str,
    cancelled: &SessionCancelToken,
) -> Result<(), SessionFlowError> {
    if cancelled.is_cancelled()
        || !sessions
            .get(session_id)
            .is_some_and(|record| record.cancelled.same_instance(cancelled))
    {
        return Err(missing_session_error(session_id));
    }
    Ok(())
}
