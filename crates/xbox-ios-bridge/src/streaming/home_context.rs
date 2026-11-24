//! 主机身份、电源事实与公共跨区发现共同装配启动上下文。
use super::{SessionCredentials, XboxStreamSettings};
use crate::cloud_access::{
    find_home_host_facts, home_host_facts, replace_home_host_facts, validate_bound_stream_access,
    HomeHostFacts, StreamingAccessContext,
};
use crate::data::extract_home_host_facts;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use xbox_streaming::session::{
    api::session::WebApiSessionGateway,
    discovery::{discover_home, ordered_home_regions},
};
use xbox_streaming::{
    compile, CompilerInput, Config, Context, SessionFlowError, SessionStartupScope, StreamingToken,
    Target,
};

struct HomeDiscoveryCache {
    scope: u64,
    created_at: std::time::Instant,
    regions: BTreeMap<String, String>,
}
static HOME_DISCOVERY: OnceLock<Mutex<Option<HomeDiscoveryCache>>> = OnceLock::new();
static FALLBACK_TURN: OnceLock<tokio::sync::Mutex<xbox_streaming::FallbackTurnProvider>> =
    OnceLock::new();

pub(super) async fn resolve_fallback_turn() -> Option<xbox_streaming::TurnServer> {
    FALLBACK_TURN
        .get_or_init(|| tokio::sync::Mutex::new(xbox_streaming::FallbackTurnProvider::new()))
        .lock()
        .await
        .get_by_target_type(Target::Home.as_str())
        .await
        .unwrap_or(None)
}

fn discovery_scope(access: &StreamingAccessContext) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    access.account_id.hash(&mut hash);
    access.bearer_token.hash(&mut hash);
    access.force_region_ip.hash(&mut hash);
    hash.finish()
}

pub(super) async fn resolve_home_context(
    mut access: StreamingAccessContext,
    credentials: &Arc<SessionCredentials>,
    target_id: &str,
    settings: &XboxStreamSettings,
    scope: &SessionStartupScope,
) -> Result<StreamingAccessContext, SessionFlowError> {
    if access.target != Target::Home {
        return Ok(access);
    }
    scope
        .run("homeContext", None, async {
            // 启动直接使用公共串流区域发现；Smartglass 电源事实在唤醒/就绪阶段更新。
            let cached =
                find_home_host_facts(&access.account_id, target_id).map_err(bridge_error)?;
            let canonical_id = cached
                .as_ref()
                .and_then(HomeHostFacts::canonical_target_id)
                .unwrap_or(target_id);
            let scope_key = discovery_scope(&access);
            let cached_region = HOME_DISCOVERY
                .get_or_init(|| Mutex::new(None))
                .lock()
                .ok()
                .and_then(|cache| {
                    cache
                        .as_ref()
                        .filter(|cache| {
                            cache.scope == scope_key && cache.created_at.elapsed().as_secs() < 300
                        })
                        .and_then(|cache| cache.regions.get(canonical_id).cloned())
                });
            let regions =
                ordered_home_regions(&access.session_access.regions, cached_region.as_deref());
            let discovered = discover_home(
                &regions,
                Some(canonical_id),
                |region| {
                    let mut config = Config::new_home_config(
                        super::normalize_game_locale(&settings.preferred_game_locale),
                        access.force_region_ip.clone(),
                        settings.home_resolution,
                    );
                    config.session.preferred_region_name = Some(region.name.clone());
                    let context = Context {
                        target: Target::Home,
                        session: access.session_access.clone(),
                        ..Default::default()
                    };
                    let token = StreamingToken {
                        gs_token: access.bearer_token.clone(),
                    };
                    let transport = credentials.transport(&access);
                    async move {
                        let output = compile(CompilerInput { config, context }).map_err(|_| {
                            xbox_webapi::WebApiError::parse("home discovery region unavailable")
                        })?;
                        WebApiSessionGateway::with_transport(transport, output.plan, token)
                            .get_remote_consoles()
                            .await
                    }
                },
                || {
                    !scope.is_cancelled()
                        && validate_bound_stream_access(credentials.handle(), &access).is_ok()
                },
            )
            .await?;
            ensure_current(credentials, &access, scope)?;
            access = credentials
                .access("home", None, Some(access.owner_generation))
                .await?;
            if let Some((region, consoles)) = discovered {
                let discovered = extract_home_host_facts(&serde_json::Value::Array(consoles))
                    .map_err(SessionFlowError::from)?;
                let mut facts = home_host_facts(&access.account_id).map_err(bridge_error)?;
                for fresh in discovered {
                    let identity = fresh.canonical_target_id().unwrap_or_default();
                    if let Some(index) = facts.iter().position(|cached| cached.matches(identity)) {
                        facts[index] = merge_home_facts(&facts[index], fresh);
                    } else {
                        facts.push(fresh);
                    }
                }
                replace_home_host_facts(&access.account_id, facts).map_err(bridge_error)?;
                access
                    .session_access
                    .regions
                    .retain(|current| current.name.eq_ignore_ascii_case(&region.name));
                if access.session_access.regions.is_empty() {
                    return Err(SessionFlowError::message(
                        "homeDiscoveryCredentialRegionChanged",
                    ));
                }
                if let Ok(mut cache) = HOME_DISCOVERY.get_or_init(|| Mutex::new(None)).lock() {
                    let scope_key = discovery_scope(&access);
                    if cache.as_ref().is_none_or(|cache| {
                        cache.scope != scope_key || cache.created_at.elapsed().as_secs() >= 300
                    }) {
                        *cache = Some(HomeDiscoveryCache {
                            scope: scope_key,
                            created_at: std::time::Instant::now(),
                            regions: BTreeMap::new(),
                        });
                    }
                    if let Some(cache) = cache.as_mut() {
                        cache.regions.insert(canonical_id.to_owned(), region.name);
                    }
                }
            }
            Ok(access)
        })
        .await
}

fn ensure_current(
    credentials: &SessionCredentials,
    access: &StreamingAccessContext,
    scope: &SessionStartupScope,
) -> Result<(), SessionFlowError> {
    if scope.is_cancelled() {
        return Err(SessionFlowError::message(
            "operationCancelled:stage=homeContext",
        ));
    }
    validate_bound_stream_access(credentials.handle(), access).map_err(bridge_error)
}

fn bridge_error(error: crate::XboxBridgeError) -> SessionFlowError {
    SessionFlowError::message(error.to_string())
}

fn merge_home_facts(cached: &HomeHostFacts, mut discovered: HomeHostFacts) -> HomeHostFacts {
    discovered.remote_console.id = cached
        .remote_console
        .id
        .clone()
        .or(discovered.remote_console.id);
    discovered.remote_console.device_id = cached
        .remote_console
        .device_id
        .clone()
        .or(discovered.remote_console.device_id);
    discovered.remote_console.server_id = discovered
        .remote_console
        .server_id
        .or(cached.remote_console.server_id.clone());
    discovered.remote_console.power_state = cached
        .remote_console
        .power_state
        .clone()
        .or(discovered.remote_console.power_state);
    discovered.remote_console.remote_management_enabled = cached
        .remote_console
        .remote_management_enabled
        .or(discovered.remote_console.remote_management_enabled);
    discovered.remote_console.console_streaming_enabled = cached
        .remote_console
        .console_streaming_enabled
        .or(discovered.remote_console.console_streaming_enabled);
    if discovered.console_addrs.is_empty() {
        discovered.console_addrs = cached.console_addrs.clone();
    }
    discovered.remote_console.console_addrs_count = discovered.console_addrs.len() as u32;
    discovered.remote_console.ready_source = Some("console-list".into());
    discovered
}

pub(super) fn smartglass_ready_candidates(
    facts: Vec<HomeHostFacts>,
) -> Vec<xbox_streaming::RemoteConsoleSnapshot> {
    let mut index = BTreeMap::new();
    for fact in facts {
        let Some(identity) = fact.canonical_target_id().map(str::to_owned) else {
            continue;
        };
        index.entry(identity).or_insert(fact.remote_console);
    }
    index
        .into_iter()
        .map(|(identity, mut console)| {
            console.id = console.id.or_else(|| Some(identity.clone()));
            console.server_id = console
                .server_id
                .or_else(|| console.id.clone())
                .or(Some(identity));
            console.ready_source = Some("smartglass".into());
            console
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbox_streaming::{HostAddr, RemoteConsoleSnapshot};

    #[test]
    fn ready_candidates_follow_pc_identity_fallback_and_deduplication() {
        let facts = vec![
            HomeHostFacts {
                remote_console: RemoteConsoleSnapshot {
                    device_id: Some("device".into()),
                    power_state: Some("On".into()),
                    ..Default::default()
                },
                console_addrs: Vec::new(),
            },
            HomeHostFacts {
                remote_console: RemoteConsoleSnapshot {
                    device_id: Some("device".into()),
                    power_state: Some("Standby".into()),
                    ..Default::default()
                },
                console_addrs: Vec::new(),
            },
        ];
        let ready = smartglass_ready_candidates(facts);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id.as_deref(), Some("device"));
        assert_eq!(ready[0].server_id.as_deref(), Some("device"));
        assert_eq!(ready[0].power_state.as_deref(), Some("On"));
        assert_eq!(ready[0].ready_source.as_deref(), Some("smartglass"));
    }

    #[test]
    fn discovered_stream_identity_keeps_smartglass_command_and_power_facts() {
        let cached = HomeHostFacts {
            remote_console: RemoteConsoleSnapshot {
                id: Some("command".into()),
                device_id: Some("device".into()),
                server_id: Some("server".into()),
                power_state: Some("Standby".into()),
                remote_management_enabled: Some(true),
                ..Default::default()
            },
            console_addrs: vec![HostAddr {
                ip: "192.0.2.1".into(),
                port: 9000,
            }],
        };
        let discovered = HomeHostFacts {
            remote_console: RemoteConsoleSnapshot {
                server_id: Some("server".into()),
                console_streaming_enabled: Some(true),
                ..Default::default()
            },
            console_addrs: vec![HostAddr {
                ip: "2001:db8::1".into(),
                port: 9001,
            }],
        };
        let merged = merge_home_facts(&cached, discovered);
        assert_eq!(merged.command_id(), Some("command"));
        assert_eq!(merged.canonical_target_id(), Some("server"));
        assert_eq!(
            merged.remote_console.power_state.as_deref(),
            Some("Standby")
        );
        assert_eq!(merged.remote_console.remote_management_enabled, Some(true));
        assert_eq!(merged.remote_console.console_streaming_enabled, Some(true));
        assert_eq!(merged.console_addrs[0].ip, "2001:db8::1");
        assert_eq!(
            merged.remote_console.ready_source.as_deref(),
            Some("console-list")
        );
    }
}
