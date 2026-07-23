use crate::{
    auth_session_from_bundle, deserialize, normalize_force_region_ip, resolve_web_token_claims,
    AuthSession, XboxBridgeError,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use xbox_auth_flow::{AuthFlow, AuthFlowSeed, RefreshAndFinalizeInput};
use xbox_streaming::policy::session::SessionAccessContext;
use xbox_streaming::policy::types::HostAddr;
use xbox_streaming::{parse_session_access_context, Target};
use xbox_streaming::{FallbackTurnProvider, RemoteConsoleSnapshot, TurnServer};

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static STREAM_ACCESS: OnceLock<Mutex<StreamAccessRegistry>> = OnceLock::new();
static HOME_HOSTS: OnceLock<Mutex<HashMap<String, Vec<HomeHostFacts>>>> = OnceLock::new();

const ACCESS_LEASE_MAX_LIFETIME_MS: u64 = 15 * 60 * 1_000;
const ACCESS_LEASE_EXPIRY_SKEW_MS: u64 = 60 * 1_000;

#[derive(Debug, Clone, uniffi::Record)]
pub struct CloudAccessResult {
    pub auth_session: AuthSession,
    pub access_handle: String,
    pub account_id: String,
    pub region_host: String,
    pub owner_generation: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HomeAccessResult {
    pub auth_session: AuthSession,
    pub access_handle: String,
    pub account_id: String,
    pub region_host: String,
    pub owner_generation: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct StreamingAccessContext {
    pub(crate) host: String,
    pub(crate) bearer_token: String,
    pub(crate) account_id: String,
    pub(crate) refresh_token: String,
    pub(crate) target: Target,
    pub(crate) session_access: SessionAccessContext,
    pub(crate) force_region_ip: String,
    pub(crate) web_uhs: String,
    pub(crate) web_token: String,
    pub(crate) owner_generation: u64,
    pub(crate) created_at_ms: u64,
    pub(crate) expires_at_ms: u64,
    pub(crate) fallback_turn: Option<TurnServer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HomeHostFacts {
    pub(crate) remote_console: RemoteConsoleSnapshot,
    pub(crate) console_addrs: Vec<HostAddr>,
}

impl HomeHostFacts {
    pub(crate) fn canonical_target_id(&self) -> Option<&str> {
        self.remote_console
            .server_id
            .as_deref()
            .or(self.remote_console.id.as_deref())
            .or(self.remote_console.device_id.as_deref())
    }

    pub(crate) fn command_id(&self) -> Option<&str> {
        self.remote_console
            .id
            .as_deref()
            .or(self.remote_console.server_id.as_deref())
            .or(self.remote_console.device_id.as_deref())
    }

    pub(crate) fn matches(&self, target_id: &str) -> bool {
        self.remote_console.server_id.as_deref() == Some(target_id)
            || self.remote_console.id.as_deref() == Some(target_id)
            || self.remote_console.device_id.as_deref() == Some(target_id)
    }
}

#[derive(Debug, Clone)]
struct StreamAccessLease {
    metadata: StreamAccessLeaseMetadata,
    context: Option<StreamingAccessContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamAccessLeaseTerminal {
    Released,
    Superseded,
    Expired,
}

#[derive(Debug, Clone)]
struct StreamAccessLeaseMetadata {
    account_id: String,
    target: Target,
    owner_generation: u64,
    created_at_ms: u64,
    expires_at_ms: u64,
    terminal: Option<StreamAccessLeaseTerminal>,
}

#[derive(Debug, Default)]
struct StreamAccessRegistry {
    leases: HashMap<String, StreamAccessLease>,
    owner_generations: HashMap<(String, String), u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LeaseMetadata {
    owner_generation: u64,
    expires_at_ms: u64,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn prepare_cloud_access(
    refresh_token: String,
    seed_json: String,
    force_region_ip: String,
) -> Result<CloudAccessResult, XboxBridgeError> {
    let seed: AuthFlowSeed = deserialize(&seed_json)?;
    let force_region_ip = normalize_force_region_ip(force_region_ip);
    let output = AuthFlow::new()
        .refresh_and_finalize(RefreshAndFinalizeInput {
            refresh_token,
            seed: seed.clone(),
            force_region_ip: force_region_ip.clone(),
            include_streaming_tokens: true,
        })
        .await
        .map_err(|error| XboxBridgeError::Authentication(error.to_string()))?;

    let created_at_ms = now_ms();
    let expires_at_ms = cap_expiry_with_web_token(
        resolve_access_expiry_ms(
            output
                .auth_bundle
                .stream_tokens
                .get("xCloudToken")
                .or_else(|| output.auth_bundle.stream_tokens.get("xcloudToken")),
            created_at_ms,
        ),
        &output.auth_bundle.web_token,
        created_at_ms,
    );
    let mut context = resolve_cloud_access_context(
        &output.auth_bundle.stream_tokens,
        &output.auth_bundle.web_token,
    )?;
    context.refresh_token = output.auth_bundle.user_token.refresh_token.clone();
    context.force_region_ip = force_region_ip;
    let access_handle = next_handle();
    let account_id = context.account_id.clone();
    let region_host = context.host.clone();
    let metadata = stream_access_registry()?.insert(
        access_handle.clone(),
        context,
        created_at_ms,
        expires_at_ms,
    );
    let auth_session = auth_session_from_bundle(output.auth_bundle, &seed)?;

    Ok(CloudAccessResult {
        auth_session,
        access_handle,
        account_id,
        region_host,
        owner_generation: metadata.owner_generation,
        expires_at_ms: metadata.expires_at_ms,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn prepare_home_access(
    refresh_token: String,
    seed_json: String,
    force_region_ip: String,
) -> Result<HomeAccessResult, XboxBridgeError> {
    let seed: AuthFlowSeed = deserialize(&seed_json)?;
    let force_region_ip = normalize_force_region_ip(force_region_ip);
    let output = AuthFlow::new()
        .refresh_and_finalize(RefreshAndFinalizeInput {
            refresh_token,
            seed: seed.clone(),
            force_region_ip: force_region_ip.clone(),
            include_streaming_tokens: true,
        })
        .await
        .map_err(|error| XboxBridgeError::Authentication(error.to_string()))?;

    let created_at_ms = now_ms();
    let expires_at_ms = cap_expiry_with_web_token(
        resolve_access_expiry_ms(
            output
                .auth_bundle
                .stream_tokens
                .get("xHomeToken")
                .or_else(|| output.auth_bundle.stream_tokens.get("xhomeToken")),
            created_at_ms,
        ),
        &output.auth_bundle.web_token,
        created_at_ms,
    );
    let mut context = resolve_home_access_context(
        &output.auth_bundle.stream_tokens,
        &output.auth_bundle.web_token,
    )?;
    context.refresh_token = output.auth_bundle.user_token.refresh_token.clone();
    context.force_region_ip = force_region_ip;
    context.fallback_turn = FallbackTurnProvider::new()
        .get_by_target_type(Target::Home.as_str())
        .await
        .unwrap_or(None);
    let access_handle = next_handle();
    let account_id = context.account_id.clone();
    let region_host = context.host.clone();
    let metadata = stream_access_registry()?.insert(
        access_handle.clone(),
        context,
        created_at_ms,
        expires_at_ms,
    );
    let auth_session = auth_session_from_bundle(output.auth_bundle, &seed)?;

    Ok(HomeAccessResult {
        auth_session,
        access_handle,
        account_id,
        region_host,
        owner_generation: metadata.owner_generation,
        expires_at_ms: metadata.expires_at_ms,
    })
}

#[uniffi::export]
pub fn release_stream_access(access_handle: String) -> Result<(), XboxBridgeError> {
    stream_access_registry()?.release(access_handle.trim(), now_ms())
}

pub(crate) fn load_stream_access(
    access_handle: &str,
) -> Result<StreamingAccessContext, XboxBridgeError> {
    stream_access_registry()?.load(access_handle.trim(), None, None, None, now_ms())
}

pub(crate) fn load_scoped_stream_access(
    access_handle: &str,
    target: Target,
    account_id: Option<&str>,
    owner_generation: Option<u64>,
) -> Result<StreamingAccessContext, XboxBridgeError> {
    stream_access_registry()?.load(
        access_handle.trim(),
        Some(target),
        account_id,
        owner_generation,
        now_ms(),
    )
}

fn resolve_cloud_access_context(
    stream_tokens: &Value,
    web_token: &Value,
) -> Result<StreamingAccessContext, XboxBridgeError> {
    let token = stream_tokens
        .get("xCloudToken")
        .or_else(|| stream_tokens.get("xcloudToken"))
        .ok_or_else(|| {
            XboxBridgeError::Authentication(cloud_token_unavailable_message(stream_tokens))
        })?;
    let data = token.get("data").unwrap_or(token);
    let session_access = parse_session_access_context(token)
        .map_err(|error| XboxBridgeError::InvalidData(error.to_string()))?;
    let bearer_token = data
        .get("gsToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("xCloud token is missing gsToken".to_string())
        })?;
    let regions = data
        .get("offeringSettings")
        .and_then(|value| value.get("regions"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("xCloud token is missing regions".to_string())
        })?;
    let region = regions
        .iter()
        .find(|item| item.get("isDefault").and_then(Value::as_bool) == Some(true))
        .or_else(|| regions.first())
        .ok_or_else(|| XboxBridgeError::InvalidData("xCloud token has no region".to_string()))?;
    let base_uri = region
        .get("baseUri")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("xCloud region is missing baseUri".to_string())
        })?;
    let host = base_uri
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(XboxBridgeError::InvalidData(
            "xCloud region host is empty".to_string(),
        ));
    }
    let claims = resolve_web_token_claims(web_token)?;
    let account_id = claims.xuid.clone().unwrap_or_else(|| claims.uhs.clone());

    Ok(StreamingAccessContext {
        host: host.to_string(),
        bearer_token: bearer_token.to_string(),
        account_id,
        refresh_token: String::new(),
        target: Target::Cloud,
        session_access,
        force_region_ip: String::new(),
        web_uhs: claims.uhs,
        web_token: claims.token,
        owner_generation: 0,
        created_at_ms: 0,
        expires_at_ms: 0,
        fallback_turn: None,
    })
}

fn resolve_home_access_context(
    stream_tokens: &Value,
    web_token: &Value,
) -> Result<StreamingAccessContext, XboxBridgeError> {
    let token = stream_tokens
        .get("xHomeToken")
        .or_else(|| stream_tokens.get("xhomeToken"))
        .ok_or_else(|| XboxBridgeError::Authentication("xHome token is unavailable".to_string()))?;
    let session_access = parse_session_access_context(token)
        .map_err(|error| XboxBridgeError::InvalidData(error.to_string()))?;
    let bearer_token = session_access.gs_token.as_deref().ok_or_else(|| {
        XboxBridgeError::InvalidData("xHome token is missing gsToken".to_string())
    })?;
    let region = session_access
        .regions
        .iter()
        .find(|region| region.is_default)
        .or_else(|| session_access.regions.first())
        .ok_or_else(|| XboxBridgeError::InvalidData("xHome token has no region".to_string()))?;
    let host = region
        .base_uri
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(XboxBridgeError::InvalidData(
            "xHome region host is empty".to_string(),
        ));
    }
    let claims = resolve_web_token_claims(web_token)?;
    let account_id = claims.xuid.clone().unwrap_or_else(|| claims.uhs.clone());

    Ok(StreamingAccessContext {
        host: host.to_string(),
        bearer_token: bearer_token.to_string(),
        account_id,
        refresh_token: String::new(),
        target: Target::Home,
        session_access,
        force_region_ip: String::new(),
        web_uhs: claims.uhs,
        web_token: claims.token,
        owner_generation: 0,
        created_at_ms: 0,
        expires_at_ms: 0,
        fallback_turn: None,
    })
}

fn cloud_token_unavailable_message(stream_tokens: &Value) -> String {
    let diagnostics = stream_tokens
        .get("_diagnostics")
        .and_then(|value| value.get("xCloudToken"));
    let force_region_applied = diagnostics
        .and_then(|value| value.get("forceRegionApplied"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut parts = vec![format!(
        "xCloud token is unavailable; forceRegionApplied={force_region_applied}"
    )];
    for offering in ["xgpuweb", "xgpuwebf2p"] {
        let Some(detail) = diagnostics.and_then(|value| value.get(offering)) else {
            continue;
        };
        let error_kind = detail
            .get("errorKind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let status_code = detail
            .get("statusCode")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string());
        let timeout = detail
            .get("timeout")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let retriable = detail
            .get("retriable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        parts.push(format!(
            "offering={offering},errorKind={error_kind},statusCode={status_code},timeout={timeout},retriable={retriable}"
        ));
    }
    parts.join("; ")
}

fn stream_access_registry(
) -> Result<std::sync::MutexGuard<'static, StreamAccessRegistry>, XboxBridgeError> {
    STREAM_ACCESS
        .get_or_init(|| Mutex::new(StreamAccessRegistry::default()))
        .lock()
        .map_err(|_| XboxBridgeError::InvalidData("stream access registry is poisoned".to_string()))
}

impl StreamAccessRegistry {
    fn insert(
        &mut self,
        handle: String,
        mut context: StreamingAccessContext,
        created_at_ms: u64,
        expires_at_ms: u64,
    ) -> LeaseMetadata {
        self.expire_leases(created_at_ms);
        let owner_key = (
            context.account_id.clone(),
            context.target.as_str().to_string(),
        );
        let generation = self
            .owner_generations
            .entry(owner_key)
            .and_modify(|value| *value = value.saturating_add(1))
            .or_insert(1);
        let generation = *generation;

        for lease in self.leases.values_mut() {
            if lease.metadata.account_id == context.account_id
                && lease.metadata.target == context.target
                && lease.metadata.terminal.is_none()
            {
                lease.revoke(StreamAccessLeaseTerminal::Superseded);
            }
        }

        context.owner_generation = generation;
        context.created_at_ms = created_at_ms;
        context.expires_at_ms = expires_at_ms;
        self.leases.insert(
            handle,
            StreamAccessLease {
                metadata: StreamAccessLeaseMetadata {
                    account_id: context.account_id.clone(),
                    target: context.target,
                    owner_generation: generation,
                    created_at_ms,
                    expires_at_ms,
                    terminal: None,
                },
                context: Some(context),
            },
        );

        LeaseMetadata {
            owner_generation: generation,
            expires_at_ms,
        }
    }

    fn load(
        &mut self,
        handle: &str,
        expected_target: Option<Target>,
        expected_account_id: Option<&str>,
        expected_owner_generation: Option<u64>,
        now_ms: u64,
    ) -> Result<StreamingAccessContext, XboxBridgeError> {
        self.expire_leases(now_ms);
        let lease = self
            .leases
            .get_mut(handle)
            .ok_or_else(|| XboxBridgeError::InvalidData("streamAccessHandleInvalid".to_string()))?;
        match lease.metadata.terminal {
            Some(StreamAccessLeaseTerminal::Expired) => {
                return Err(XboxBridgeError::InvalidData(
                    "streamAccessExpired".to_string(),
                ));
            }
            Some(StreamAccessLeaseTerminal::Released | StreamAccessLeaseTerminal::Superseded) => {
                return Err(XboxBridgeError::InvalidData(
                    "streamAccessRevoked".to_string(),
                ));
            }
            None => {}
        }
        if expected_target.is_some_and(|target| target != lease.metadata.target) {
            return Err(XboxBridgeError::InvalidData(
                "streamAccessTargetMismatch".to_string(),
            ));
        }
        if expected_account_id.is_some_and(|account| account != lease.metadata.account_id) {
            return Err(XboxBridgeError::InvalidData(
                "streamAccessOwnerMismatch".to_string(),
            ));
        }
        if expected_owner_generation
            .is_some_and(|generation| generation != lease.metadata.owner_generation)
        {
            return Err(XboxBridgeError::InvalidData(
                "streamAccessGenerationMismatch".to_string(),
            ));
        }
        let owner_key = (
            lease.metadata.account_id.clone(),
            lease.metadata.target.as_str().to_string(),
        );
        if self.owner_generations.get(&owner_key).copied() != Some(lease.metadata.owner_generation)
        {
            return Err(XboxBridgeError::InvalidData(
                "streamAccessGenerationStale".to_string(),
            ));
        }
        lease.context.clone().ok_or_else(|| {
            XboxBridgeError::InvalidData("streamAccessContextUnavailable".to_string())
        })
    }

    fn release(&mut self, handle: &str, now_ms: u64) -> Result<(), XboxBridgeError> {
        self.expire_leases(now_ms);
        let lease = self
            .leases
            .get_mut(handle)
            .ok_or_else(|| XboxBridgeError::InvalidData("streamAccessHandleInvalid".to_string()))?;
        if lease.metadata.terminal.is_none() {
            lease.revoke(StreamAccessLeaseTerminal::Released);
        }
        Ok(())
    }

    fn expire_leases(&mut self, now_ms: u64) {
        for lease in self.leases.values_mut() {
            let elapsed_ms = now_ms.saturating_sub(lease.metadata.created_at_ms);
            let lifetime_ms = lease
                .metadata
                .expires_at_ms
                .saturating_sub(lease.metadata.created_at_ms);
            if lease.metadata.terminal.is_none()
                && elapsed_ms.saturating_add(ACCESS_LEASE_EXPIRY_SKEW_MS) >= lifetime_ms
            {
                lease.revoke(StreamAccessLeaseTerminal::Expired);
            }
        }
    }
}

impl StreamAccessLease {
    fn revoke(&mut self, terminal: StreamAccessLeaseTerminal) {
        self.metadata.terminal = Some(terminal);
        self.context = None;
    }
}

pub(crate) fn replace_home_host_facts(
    account_id: &str,
    facts: Vec<HomeHostFacts>,
) -> Result<(), XboxBridgeError> {
    HOME_HOSTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| XboxBridgeError::InvalidData("home host registry is poisoned".to_string()))?
        .insert(account_id.to_string(), facts);
    Ok(())
}

pub(crate) fn home_host_facts(account_id: &str) -> Result<Vec<HomeHostFacts>, XboxBridgeError> {
    Ok(HOME_HOSTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| XboxBridgeError::InvalidData("home host registry is poisoned".to_string()))?
        .get(account_id)
        .cloned()
        .unwrap_or_default())
}

pub(crate) fn find_home_host_facts(
    account_id: &str,
    target_id: &str,
) -> Result<Option<HomeHostFacts>, XboxBridgeError> {
    Ok(home_host_facts(account_id)?
        .into_iter()
        .find(|facts| facts.matches(target_id)))
}

fn resolve_access_expiry_ms(token: Option<&Value>, created_at_ms: u64) -> u64 {
    let capped_expiry = created_at_ms.saturating_add(ACCESS_LEASE_MAX_LIFETIME_MS);
    let token_expiry = token.and_then(|token| {
        let data = token.get("data").unwrap_or(token);
        let create_time = token
            .get("_objectCreateTime")
            .or_else(|| data.get("_objectCreateTime"))
            .and_then(parse_u64_value)?;
        let duration_seconds = data
            .get("durationInSeconds")
            .or_else(|| token.get("durationInSeconds"))
            .or_else(|| data.get("duration_in_seconds"))
            .and_then(parse_u64_value)?;
        Some(create_time.saturating_add(duration_seconds.saturating_mul(1_000)))
    });
    token_expiry
        .filter(|expiry| *expiry > created_at_ms)
        .map(|expiry| expiry.min(capped_expiry))
        .unwrap_or(capped_expiry)
}

fn cap_expiry_with_web_token(stream_expiry_ms: u64, web_token: &Value, created_at_ms: u64) -> u64 {
    let web_expiry_ms = web_token
        .get("data")
        .and_then(|data| data.get("NotAfter"))
        .or_else(|| web_token.get("NotAfter"))
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp_millis())
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > created_at_ms);
    web_expiry_ms
        .map(|value| stream_expiry_ms.min(value))
        .unwrap_or(stream_expiry_ms)
}

fn parse_u64_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn next_handle() -> String {
    let sequence = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    format!("stream-{sequence:016x}-{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_context(account_id: &str, target: Target) -> StreamingAccessContext {
        StreamingAccessContext {
            host: "stream.example.com".to_string(),
            bearer_token: "gs-token".to_string(),
            account_id: account_id.to_string(),
            refresh_token: "refresh-token".to_string(),
            target,
            session_access: SessionAccessContext {
                gs_token: Some("gs-token".to_string()),
                regions: vec![],
            },
            force_region_ip: String::new(),
            web_uhs: "uhs".to_string(),
            web_token: "web-token".to_string(),
            owner_generation: 0,
            created_at_ms: 0,
            expires_at_ms: 0,
            fallback_turn: None,
        }
    }

    #[test]
    fn resolves_default_region_and_stable_xid() {
        let context = resolve_cloud_access_context(
            &json!({
                "xCloudToken": {
                    "data": {
                        "gsToken": "cloud-token",
                        "offeringSettings": {
                            "regions": [
                                { "baseUri": "https://first.example.com" },
                                {
                                    "isDefault": true,
                                    "baseUri": "https://default.example.com/"
                                }
                            ]
                        }
                    }
                }
            }),
            &json!({
                "data": {
                    "Token": "web-token",
                    "DisplayClaims": {
                        "xui": [{ "uhs": "volatile", "xid": "stable-xid" }]
                    }
                }
            }),
        )
        .expect("cloud access should resolve");

        assert_eq!(context.host, "default.example.com");
        assert_eq!(context.bearer_token, "cloud-token");
        assert_eq!(context.account_id, "stable-xid");
    }

    #[test]
    fn missing_cloud_token_is_rejected() {
        let result = resolve_cloud_access_context(
            &json!({
                "_diagnostics": {
                    "xCloudToken": {
                        "xgpuweb": {
                            "errorKind": "http",
                            "statusCode": 403,
                            "timeout": false,
                            "retriable": false
                        },
                        "xgpuwebf2p": {
                            "errorKind": "network",
                            "statusCode": null,
                            "timeout": true,
                            "retriable": true
                        },
                        "forceRegionApplied": false
                    }
                }
            }),
            &json!({}),
        );
        let error = result
            .expect_err("missing cloud token should fail")
            .to_string();
        assert!(error.contains("forceRegionApplied=false"));
        assert!(error.contains("offering=xgpuweb,errorKind=http,statusCode=403"));
        assert!(error.contains("offering=xgpuwebf2p,errorKind=network,statusCode=none"));
    }

    #[test]
    fn resolves_home_access_from_xhome_token() {
        let context = resolve_home_access_context(
            &json!({
                "xHomeToken": {
                    "data": {
                        "gsToken": "home-token",
                        "offeringSettings": {
                            "regions": [{
                                "name": "HOME",
                                "baseUri": "https://home.example.com/",
                                "isDefault": true
                            }]
                        }
                    }
                }
            }),
            &json!({
                "data": {
                    "Token": "web-token",
                    "DisplayClaims": {
                        "xui": [{ "uhs": "volatile", "xid": "stable-xid" }]
                    }
                }
            }),
        )
        .expect("home access should resolve");

        assert_eq!(context.target, Target::Home);
        assert_eq!(context.host, "home.example.com");
        assert_eq!(context.bearer_token, "home-token");
        assert_eq!(context.account_id, "stable-xid");
    }

    #[test]
    fn scoped_lease_rejects_target_and_cross_account_mismatch() {
        let mut registry = StreamAccessRegistry::default();
        let first = registry.insert(
            "cloud-lease".to_string(),
            fixture_context("account-a", Target::Cloud),
            100,
            100_000,
        );
        assert_eq!(first.owner_generation, 1);
        let target_error = registry
            .load("cloud-lease", Some(Target::Home), None, None, 100)
            .expect_err("target mismatch");
        assert!(target_error
            .to_string()
            .contains("streamAccessTargetMismatch"));
        let owner_error = registry
            .load(
                "cloud-lease",
                Some(Target::Cloud),
                Some("account-b"),
                Some(first.owner_generation),
                100,
            )
            .expect_err("owner mismatch");
        assert!(owner_error
            .to_string()
            .contains("streamAccessOwnerMismatch"));
    }

    #[test]
    fn new_owner_generation_revokes_stale_lease() {
        let mut registry = StreamAccessRegistry::default();
        registry.insert(
            "old".to_string(),
            fixture_context("account-a", Target::Cloud),
            100,
            100_000,
        );
        let second = registry.insert(
            "new".to_string(),
            fixture_context("account-a", Target::Cloud),
            200,
            100_000,
        );
        assert_eq!(second.owner_generation, 2);
        let stale = registry
            .load("old", Some(Target::Cloud), None, None, 200)
            .expect_err("old generation must be revoked");
        assert!(stale.to_string().contains("streamAccessRevoked"));
        let old = registry.leases.get("old").expect("old tombstone");
        assert!(old.context.is_none());
        assert_eq!(old.metadata.account_id, "account-a");
        assert_eq!(old.metadata.target, Target::Cloud);
        assert_eq!(old.metadata.owner_generation, 1);
        assert_eq!(old.metadata.created_at_ms, 100);
        assert_eq!(old.metadata.expires_at_ms, 100_000);
        assert_eq!(
            old.metadata.terminal,
            Some(StreamAccessLeaseTerminal::Superseded)
        );
        assert!(registry
            .load("new", Some(Target::Cloud), Some("account-a"), Some(2), 200)
            .is_ok());
    }

    #[test]
    fn lease_expiry_discards_context_and_retains_non_sensitive_tombstone() {
        let mut registry = StreamAccessRegistry::default();
        registry.insert(
            "expiring".to_string(),
            fixture_context("account-a", Target::Cloud),
            100,
            1_000,
        );
        let expired = registry
            .load("expiring", None, None, None, 940)
            .expect_err("expiry skew should reject");
        assert!(expired.to_string().contains("streamAccessExpired"));
        let tombstone = registry.leases.get("expiring").expect("expiry tombstone");
        assert!(tombstone.context.is_none());
        assert_eq!(tombstone.metadata.account_id, "account-a");
        assert_eq!(tombstone.metadata.target, Target::Cloud);
        assert_eq!(tombstone.metadata.owner_generation, 1);
        assert_eq!(tombstone.metadata.created_at_ms, 100);
        assert_eq!(tombstone.metadata.expires_at_ms, 1_000);
        assert_eq!(
            tombstone.metadata.terminal,
            Some(StreamAccessLeaseTerminal::Expired)
        );
    }

    #[test]
    fn lease_release_discards_context_and_is_idempotent() {
        let mut registry = StreamAccessRegistry::default();
        registry.insert(
            "released".to_string(),
            fixture_context("account-a", Target::Cloud),
            100,
            100_000,
        );
        registry
            .release("released", 300)
            .expect("release should succeed");
        registry
            .release("released", 301)
            .expect("release should be idempotent");
        let revoked = registry
            .load("released", None, None, None, 302)
            .expect_err("released lease must reject");
        assert!(revoked.to_string().contains("streamAccessRevoked"));
        let tombstone = registry.leases.get("released").expect("release tombstone");
        assert!(tombstone.context.is_none());
        assert_eq!(tombstone.metadata.account_id, "account-a");
        assert_eq!(tombstone.metadata.target, Target::Cloud);
        assert_eq!(tombstone.metadata.owner_generation, 1);
        assert_eq!(tombstone.metadata.created_at_ms, 100);
        assert_eq!(tombstone.metadata.expires_at_ms, 100_000);
        assert_eq!(
            tombstone.metadata.terminal,
            Some(StreamAccessLeaseTerminal::Released)
        );
    }

    #[test]
    fn expiry_is_bounded_even_when_token_duration_is_long() {
        let created_at = 1_000;
        let expiry = resolve_access_expiry_ms(
            Some(&json!({
                "_objectCreateTime": created_at,
                "data": {"durationInSeconds": 86_400}
            })),
            created_at,
        );
        assert_eq!(expiry, created_at + ACCESS_LEASE_MAX_LIFETIME_MS);
    }

    #[test]
    fn web_token_not_after_caps_streaming_lease_expiry() {
        let created_at = 1_000;
        let stream_expiry = created_at + ACCESS_LEASE_MAX_LIFETIME_MS;
        let expiry = cap_expiry_with_web_token(
            stream_expiry,
            &json!({"data": {"NotAfter": "1970-01-01T00:10:00Z"}}),
            created_at,
        );
        assert_eq!(expiry, 600_000);
    }
}
