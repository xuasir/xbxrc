//! 信令事务：阶段期限、协商串行化、当前资源身份与增量候选提交。
use super::*;
use crate::session::signaling::state::media_ice_credentials;

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
                    let _ice = mutation.ice.lock().await;
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    let api = self.create_signaling_api(&record.plan).await?;
                    let prior = self
                        .inner
                        .signaling
                        .read()
                        .await
                        .get(session_id)
                        .cloned()
                        .unwrap_or_default();
                    let media = channel != Some("chat");
                    let channel_key = if media { "media" } else { "chat" };
                    let credentials = media_ice_credentials(sdp);
                    let new_ice = media
                        && (restart
                            || credentials.is_empty()
                            || prior.media_offer_credentials.as_ref() != Some(&credentials));
                    // Peer 重建的 restart=false 也会更换 ICE 凭证；同 Peer 的画质协商允许复用 answer。
                    let require_fresh_answer = new_ice || restart || !media;
                    let previous = if require_fresh_answer && (prior.offer_started || restart) {
                        api.get_sdp_exchange_response(session_id)
                            .await
                            .map_err(map_webapi_error)?
                            .or_else(|| prior.answers.get(channel_key).cloned())
                    } else {
                        None
                    };
                    let baseline = if new_ice
                        && (prior.media_offer_credentials.is_some()
                            || prior.last_polled_ice.is_some())
                    {
                        api.get_ice_exchange_response(session_id)
                            .await
                            .map_err(map_webapi_error)?
                            .or(prior.last_polled_ice.clone())
                            .or(prior.baseline_ice.clone())
                    } else {
                        None
                    };
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    {
                        let sessions = self.inner.sessions.read().await;
                        ensure_record_current(&sessions, session_id, &record.cancelled)?;
                        let mut signaling = self.inner.signaling.write().await;
                        let state = signaling.entry(session_id.to_owned()).or_default();
                        state.offer_started = true;
                        if new_ice {
                            state.begin_media_ice(credentials, baseline);
                        }
                    }
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
                        if let Some(answer) = filter_stale_offer_response(
                            answer,
                            previous.as_ref(),
                            require_fresh_answer,
                        )
                        .filter(|answer| {
                            !require_fresh_answer || prior.answers.get(channel_key) != Some(answer)
                        }) {
                            let sessions = self.inner.sessions.read().await;
                            ensure_record_current(&sessions, session_id, &record.cancelled)?;
                            let mut signaling = self.inner.signaling.write().await;
                            let state = signaling.entry(session_id.to_owned()).or_default();
                            if media {
                                state.apply_media_answer(&answer);
                            }
                            state.answers.insert(channel_key, answer.clone());
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
        _restart: bool,
    ) -> Result<(), SessionFlowError> {
        let record = self
            .get_session_record(session_id)
            .await
            .ok_or_else(|| missing_session_error(session_id))?;
        let generation = self.ice_generation(session_id).await;
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
                    self.ensure_ice_generation(session_id, generation).await?;
                    let api = self.create_signaling_api(&record.plan).await?;
                    api.send_ice(session_id, candidates)
                        .await
                        .map_err(map_webapi_error)?;
                    self.ensure_current_session(session_id, &record.cancelled)
                        .await?;
                    Ok(())
                },
            )
            .await
    }

    pub async fn poll_ice(
        &self,
        session_id: &str,
        _restart: bool,
    ) -> Result<Vec<IceCandidate>, SessionFlowError> {
        let record = self
            .get_session_record(session_id)
            .await
            .ok_or_else(|| missing_session_error(session_id))?;
        let generation = self.ice_generation(session_id).await;
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
                    self.ensure_ice_generation(session_id, generation).await?;
                    let api = self.create_signaling_api(&record.plan).await?;
                    let poll_started_at = tokio::time::Instant::now();
                    let needs_configuration = if record.plan.session.target.is_home() {
                        // 请求前只读取状态；旧会话的迟到响应无法创建或覆盖新状态。
                        self.inner.signaling.read().await.get(session_id).cloned()
                            .unwrap_or_default().begin_configuration_poll(poll_started_at)
                    } else { false };
                    let (response, configuration) = tokio::join!(
                        api.get_ice_exchange_response(session_id),
                        async {
                            if !needs_configuration { return Ok(None); }
                            let session_api = self.create_session_api(&record.plan).await?;
                            match tokio::time::timeout(std::time::Duration::from_millis(1500), session_api.get_configuration(session_id)).await {
                                Ok(result) => result.map_err(map_webapi_error),
                                Err(_) => Ok(None),
                            }
                        }
                    );
                    let response = response.map_err(map_webapi_error)?;
                    if let Err(error) = &configuration {
                        if matches!(error.status, Some(401 | 403)) { return Err(error.clone()); }
                    }
                    // 会话读锁与信令写锁共同保护每次响应提交，包括配置事实与补候选。
                    let sessions = self.inner.sessions.read().await;
                    ensure_record_current(&sessions, session_id, &record.cancelled)?;
                    let mut signaling = self.inner.signaling.write().await;
                    let state = signaling.entry(session_id.to_owned()).or_default();
                    if state.generation != generation {
                        return Err(SessionFlowError::message("signalingSuperseded"));
                    }
                    let response = state.filter_ice(response);
                    let response = if record.plan.session.target.is_home() {
                        let was_pending = state.configuration_pending();
                        state.begin_configuration_poll(poll_started_at);
                        let unavailable = configuration.as_ref().err().is_some_and(|error| matches!(error.status, Some(404 | 405 | 501)));
                        if needs_configuration {
                            state.observe_configuration(configuration.as_ref().ok().and_then(Option::as_ref), unavailable);
                        }
                        if needs_configuration || was_pending != state.configuration_pending() {
                            self.inner.provider.on_session_lifecycle_event("homeConfigurationObserved", Some(session_id), serde_json::json!({
                                "generation": generation, "configurationGeneration": state.configuration_generation,
                                "configurationResolved": state.home_configuration.is_some(), "configurationPending": state.configuration_pending(),
                                "addressCount": state.home_configuration.as_ref().map_or(0, |config| config.addresses.len()),
                                "invalidAddressCount": state.home_configuration.as_ref().map_or(0, |config| config.invalid_addresses),
                                "source": if state.home_configuration.is_some() { "session-configuration" } else if state.configuration_pending() { "pending" } else { "console-list" },
                                "reason": if state.home_configuration.is_some() { "resolved" } else if unavailable { "unsupported" } else if !state.configuration_pending() { "deadline" } else if configuration.is_err() { "request-failed" } else { "pending" },
                            }));
                        }
                        let fallback = if record.plan.negotiation.inject_console_addrs { record.plan.negotiation.console_addrs.as_slice() } else { &[] };
                        state.supplement_home_ice(response, fallback, record.plan.negotiation.prefer_ipv6)
                    } else { response };
                    resolve_polled_ice_result(session_id, response, true)
                },
            )
            .await
    }

    pub async fn home_configuration_pending(&self, session_id: &str) -> bool {
        let home = self
            .get_session_record(session_id)
            .await
            .is_some_and(|record| record.plan.session.target.is_home());
        home && self
            .inner
            .signaling
            .read()
            .await
            .get(session_id)
            .is_some_and(|state| state.configuration_pending())
    }

    async fn ensure_current_session(
        &self,
        session_id: &str,
        cancelled: &SessionCancelToken,
    ) -> Result<(), SessionFlowError> {
        ensure_record_current(&*self.inner.sessions.read().await, session_id, cancelled)
    }

    async fn ice_generation(&self, session_id: &str) -> u64 {
        self.inner
            .signaling
            .read()
            .await
            .get(session_id)
            .map_or(0, |state| state.generation)
    }

    async fn ensure_ice_generation(
        &self,
        session_id: &str,
        generation: u64,
    ) -> Result<(), SessionFlowError> {
        if self.ice_generation(session_id).await != generation {
            return Err(SessionFlowError::message("signalingSuperseded"));
        }
        Ok(())
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
