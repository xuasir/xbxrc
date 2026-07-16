use crate::error::AuthFlowError;
use crate::types::{
    AuthBundle, AuthFlowSeed, BuildDownstreamTokensInput, BuildDownstreamTokensOutput,
    CompleteOAuthLoginInput, CompleteOAuthLoginOutput, FlowSisuTokenData, FlowTokenDetails,
    PendingOAuthLogin, RefreshAndFinalizeInput, RefreshAndFinalizeOutput, StartOAuthLoginInput,
    StartOAuthLoginOutput,
};
use serde_json::{json, Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;
use xbox_webapi::AuthApi;

const XAL_TITLE_ID: &str = "000000004c20a908";
const XAL_DEVICE_VERSION: &str = "15.0";

pub struct AuthFlow {
    auth_api: AuthApi,
}

impl AuthFlow {
    pub fn new() -> Self {
        Self {
            auth_api: AuthApi::new(),
        }
    }

    pub fn with_api(auth_api: AuthApi) -> Self {
        Self { auth_api }
    }

    pub async fn start_oauth_login(
        &self,
        input: StartOAuthLoginInput,
    ) -> Result<StartOAuthLoginOutput, AuthFlowError> {
        // 这里只封装 Xbox 协议握手，调用侧自行决定如何持久化 seed/pending。
        let jwt_keys = xbox_webapi::generate_ecdsa_keypair().map_err(AuthFlowError::Protocol)?;
        let private_jwk = jwt_keys
            .private_jwk
            .ok_or(AuthFlowError::MissingPrivateJwk)?;

        let device_uuid = uuid::Uuid::new_v4().to_string();
        let serial_number = uuid::Uuid::new_v4().to_string();
        let device_token = self
            .auth_api
            .get_device_token(
                &input.title_id,
                &device_uuid,
                &serial_number,
                &input.device_version,
                &private_jwk,
            )
            .await?
            .Token;

        let code_challenge = xbox_webapi::create_code_challenge();
        let oauth_state = xbox_webapi::get_random_state();
        let sisu_auth = self
            .auth_api
            .sisu_authenticate(
                &device_token,
                &code_challenge.value,
                &code_challenge.method,
                &oauth_state,
                &private_jwk,
            )
            .await?;
        let oauth_url = sisu_auth.msa_oauth_redirect.clone();
        let sisu_auth_value = serde_json::to_value(&sisu_auth)
            .map_err(|error| AuthFlowError::Protocol(error.to_string()))?;

        Ok(StartOAuthLoginOutput {
            oauth_url,
            oauth_state: oauth_state.clone(),
            pending: PendingOAuthLogin {
                redirect_flow: xbox_webapi::XalRedirectFlow {
                    sisu_auth: sisu_auth_value,
                    state: oauth_state,
                    code_challenge,
                },
            },
            seed: AuthFlowSeed { private_jwk },
        })
    }

    pub async fn complete_oauth_login(
        &self,
        input: CompleteOAuthLoginInput,
    ) -> Result<CompleteOAuthLoginOutput, AuthFlowError> {
        let url = Url::parse(&input.callback_url)
            .map_err(|error| AuthFlowError::InvalidCallbackUrl(error.to_string()))?;

        if url.query_pairs().any(|(key, _)| key == "error") {
            return Err(AuthFlowError::CallbackContainsError);
        }

        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.to_string())
            .ok_or(AuthFlowError::MissingCallbackCode)?;

        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.to_string())
            .ok_or(AuthFlowError::MissingCallbackState)?;

        if state != input.pending.redirect_flow.state {
            return Err(AuthFlowError::StateMismatch);
        }

        let user_token = self
            .auth_api
            .exchange_code_for_token(&code, &input.pending.redirect_flow.code_challenge.verifier)
            .await?;
        let auth_bundle = self
            .finalize_login(
                &user_token,
                &input.seed.private_jwk,
                &input.force_region_ip,
                input.include_streaming_tokens,
            )
            .await?;

        Ok(CompleteOAuthLoginOutput { auth_bundle })
    }

    pub async fn refresh_and_finalize(
        &self,
        input: RefreshAndFinalizeInput,
    ) -> Result<RefreshAndFinalizeOutput, AuthFlowError> {
        let refreshed_user_token = self
            .auth_api
            .refresh_user_token(&input.refresh_token)
            .await?;
        let auth_bundle = self
            .finalize_login(
                &refreshed_user_token,
                &input.seed.private_jwk,
                &input.force_region_ip,
                input.include_streaming_tokens,
            )
            .await?;

        Ok(RefreshAndFinalizeOutput {
            auth_bundle,
            refreshed_user_token,
        })
    }

    pub async fn build_downstream_tokens(
        &self,
        input: BuildDownstreamTokensInput,
    ) -> Result<BuildDownstreamTokensOutput, AuthFlowError> {
        let auth_bundle = self
            .build_auth_bundle(
                &input.user_token,
                input.sisu_token,
                &input.seed.private_jwk,
                &input.force_region_ip,
                true,
            )
            .await?;

        Ok(BuildDownstreamTokensOutput { auth_bundle })
    }

    pub async fn transfer_token(
        &self,
        input: crate::types::TransferTokenInput,
    ) -> Result<crate::types::TransferTokenOutput, AuthFlowError> {
        let transfer_token = self
            .auth_api
            .get_cloud_transfer_token(&input.refresh_token)
            .await?;

        Ok(crate::types::TransferTokenOutput { transfer_token })
    }

    // 这里把 user token 之后的协议链路整合起来，方便回调链路和后续 refresh 复用。
    async fn finalize_login(
        &self,
        user_token: &xbox_webapi::OAuthTokenResponse,
        private_jwk: &Value,
        force_region_ip: &str,
        include_streaming_tokens: bool,
    ) -> Result<AuthBundle, AuthFlowError> {
        let device_uuid = uuid::Uuid::new_v4().to_string();
        let serial_number = uuid::Uuid::new_v4().to_string();
        let device_token = self
            .auth_api
            .get_device_token(
                XAL_TITLE_ID,
                &device_uuid,
                &serial_number,
                XAL_DEVICE_VERSION,
                private_jwk,
            )
            .await?
            .Token;

        let sisu_auth_res = self
            .auth_api
            .sisu_authorize(&user_token.access_token, &device_token, private_jwk)
            .await?;

        let title_token = sisu_auth_res
            .title_token
            .ok_or(AuthFlowError::MissingSisuField("TitleToken"))?;
        let user_sisu_token = sisu_auth_res
            .user_token
            .ok_or(AuthFlowError::MissingSisuField("UserToken"))?;
        let authorization_token = sisu_auth_res
            .authorization_token
            .ok_or(AuthFlowError::MissingSisuField("AuthorizationToken"))?;
        let resolved_device_token = sisu_auth_res
            .device_token
            .unwrap_or_else(|| device_token.clone());

        let sisu_token = FlowSisuTokenData {
            device_token: resolved_device_token,
            title_token: convert_token_details(title_token)?,
            user_token: convert_token_details(user_sisu_token)?,
            authorization_token: convert_token_details(authorization_token)?,
        };

        self.build_auth_bundle(
            user_token,
            sisu_token,
            private_jwk,
            force_region_ip,
            include_streaming_tokens,
        )
        .await
    }

    // 这里统一收敛 XSTS/Web/streaming token 生成，供 refresh 与 skip-refresh 复用。
    async fn build_auth_bundle(
        &self,
        user_token: &xbox_webapi::OAuthTokenResponse,
        sisu_token: FlowSisuTokenData,
        private_jwk: &Value,
        force_region_ip: &str,
        include_streaming_tokens: bool,
    ) -> Result<AuthBundle, AuthFlowError> {
        let user_token_str = sisu_token.user_token.token.as_deref().ok_or_else(|| {
            AuthFlowError::Protocol("Sisu response missing user token string".to_string())
        })?;

        let web_token_resp = self
            .auth_api
            .xsts_authorize(user_token_str, "http://xboxlive.com", private_jwk)
            .await?;
        let token_update_time = now_ms();
        let stream_tokens = if include_streaming_tokens {
            self.build_stream_tokens(
                user_token_str,
                private_jwk,
                force_region_ip,
                token_update_time,
            )
            .await?
        } else {
            Map::new()
        };

        Ok(AuthBundle {
            user_token: user_token.clone(),
            sisu_token,
            web_token: json!({ "data": web_token_resp }),
            stream_tokens: Value::Object(stream_tokens.clone()),
            app_level: if !include_streaming_tokens {
                0
            } else if has_xcloud_token(&stream_tokens) {
                2
            } else {
                1
            },
            token_update_time,
        })
    }

    async fn build_stream_tokens(
        &self,
        user_token_str: &str,
        private_jwk: &Value,
        force_region_ip: &str,
        token_update_time: u64,
    ) -> Result<Map<String, Value>, AuthFlowError> {
        let gssv_token = self
            .auth_api
            .xsts_authorize(user_token_str, "http://gssv.xboxlive.com/", private_jwk)
            .await?
            .Token;

        // xHome 会受区域 IP 影响，这里继续保留桌面端既有语义。
        let xhome_token = self
            .auth_api
            .get_streaming_token(&gssv_token, "xhome", force_region_ip)
            .await?;
        let mut stream_tokens = Map::new();
        stream_tokens.insert(
            "xHomeToken".to_string(),
            json!({
                "_objectCreateTime": token_update_time as i64,
                "data": xhome_token.data
            }),
        );

        let xgpuweb_result = self
            .auth_api
            .get_streaming_token(&gssv_token, "xgpuweb", force_region_ip)
            .await;
        match xgpuweb_result {
            Ok(token) => {
                stream_tokens.insert(
                    "xCloudToken".to_string(),
                    json!({
                        "_objectCreateTime": token_update_time as i64,
                        "data": token.data
                    }),
                );
            }
            Err(xgpuweb_error) => {
                let xgpuweb_diagnostics = streaming_token_error_diagnostics(&xgpuweb_error);
                let xgpuweb_error = xgpuweb_error.to_string();
                match self
                    .auth_api
                    .get_streaming_token(&gssv_token, "xgpuwebf2p", force_region_ip)
                    .await
                {
                    Ok(token) => {
                        log::warn!(
                            "Auth: xgpuweb token failed, using xgpuwebf2p fallback: {}",
                            xgpuweb_error
                        );
                        stream_tokens.insert(
                            "xCloudToken".to_string(),
                            json!({
                                "_objectCreateTime": token_update_time as i64,
                                "data": token.data
                            }),
                        );
                    }
                    Err(xgpuwebf2p_error) => {
                        let xgpuwebf2p_diagnostics =
                            streaming_token_error_diagnostics(&xgpuwebf2p_error);
                        let xgpuwebf2p_error = xgpuwebf2p_error.to_string();
                        log::warn!(
                            "Auth: xCloud token unavailable, xgpuweb={}, xgpuwebf2p={}",
                            xgpuweb_error,
                            xgpuwebf2p_error
                        );
                        let force_region_ip = force_region_ip.trim();
                        stream_tokens.insert(
                            "_diagnostics".to_string(),
                            json!({
                                "xCloudToken": {
                                    "xgpuweb": xgpuweb_diagnostics,
                                    "xgpuwebf2p": xgpuwebf2p_diagnostics,
                                    "forceRegionApplied": !force_region_ip.is_empty(),
                                },
                            }),
                        );
                    }
                }
            }
        }

        Ok(stream_tokens)
    }
}

fn streaming_token_error_diagnostics(error: &xbox_webapi::WebApiError) -> Value {
    let error_kind = match error {
        xbox_webapi::WebApiError::Network { .. } => "network",
        xbox_webapi::WebApiError::Http { .. } => "http",
        xbox_webapi::WebApiError::Parse { .. } => "parse",
        xbox_webapi::WebApiError::Auth { .. } => "auth",
    };
    let timeout = match error {
        xbox_webapi::WebApiError::Network { message, .. } => {
            let message = message.to_ascii_lowercase();
            message.contains("timeout") || message.contains("timed out")
        }
        xbox_webapi::WebApiError::Http { status, .. } => *status == 408 || *status == 504,
        _ => false,
    };

    json!({
        "errorKind": error_kind,
        "statusCode": error.to_status_code(),
        "timeout": timeout,
        "retriable": error.is_retriable(),
    })
}

impl Default for AuthFlow {
    fn default() -> Self {
        Self::new()
    }
}

fn convert_token_details<T>(
    source: xbox_webapi::TokenDetails<T>,
) -> Result<FlowTokenDetails, AuthFlowError>
where
    T: serde::Serialize,
{
    Ok(FlowTokenDetails {
        issue_instant: Some(source.issue_instant),
        not_after: Some(source.not_after),
        token: Some(source.Token),
        display_claims: serde_json::to_value(source.display_claims)
            .map_err(|error| AuthFlowError::Protocol(error.to_string()))?,
    })
}

fn has_xcloud_token(stream_tokens: &Map<String, Value>) -> bool {
    stream_tokens
        .get("xCloudToken")
        .is_some_and(|value| !value.is_null())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
