use super::ice::IceCandidate;
use crate::policy::negotiation::console::{
    candidate_endpoint_key, console_candidates, parse_console_configuration,
    remote_transport_bindings, ConsoleConfiguration, RemoteTransportBinding,
};
use crate::policy::types::HostAddr;
use crate::session::api::signaling::AnswerPayload;
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionSignalingState {
    pub generation: u64,
    pub offer_started: bool,
    pub answers: HashMap<&'static str, AnswerPayload>,
    pub media_offer_credentials: Option<Vec<String>>,
    pub last_polled_ice: Option<Vec<IceCandidate>>,
    pub baseline_ice: Option<Vec<IceCandidate>>,
    remote_ufrags: Vec<String>,
    remote_bindings: Vec<RemoteTransportBinding>,
    pub(crate) home_configuration: Option<ConsoleConfiguration>,
    pub(crate) configuration_generation: u64,
    configuration_started_at: Option<tokio::time::Instant>,
    configuration_finished: bool,
    supplement_keys: BTreeSet<String>,
    remote_endpoint_keys: BTreeSet<String>,
    pending_ends: Vec<IceCandidate>,
}

impl SessionSignalingState {
    pub fn begin_media_ice(
        &mut self,
        credentials: Vec<String>,
        baseline: Option<Vec<IceCandidate>>,
    ) {
        self.generation += 1;
        self.media_offer_credentials = Some(credentials);
        self.baseline_ice = baseline;
        self.last_polled_ice = None;
        self.home_configuration = None;
        self.configuration_started_at = None;
        self.configuration_finished = false;
        self.supplement_keys.clear();
        self.remote_endpoint_keys.clear();
        self.pending_ends.clear();
        self.remote_bindings.clear();
    }

    pub fn apply_media_answer(&mut self, answer: &AnswerPayload) {
        let ufrags = sdp_attributes(&answer.sdp, &["a=ice-ufrag:"]);
        // 新 answer 明确复用远端 ICE 凭证时，同地址候选仍属于当前传输。
        if !ufrags.is_empty() && ufrags == self.remote_ufrags {
            self.baseline_ice = None;
        }
        self.remote_ufrags = ufrags;
        self.remote_bindings = remote_transport_bindings(&answer.sdp);
    }

    pub(crate) fn configuration_pending(&self) -> bool {
        !self.configuration_finished
    }
    pub(crate) fn begin_configuration_poll(&mut self, now: tokio::time::Instant) -> bool {
        if self.configuration_finished {
            return false;
        }
        let started = self.configuration_started_at.get_or_insert(now);
        if started.elapsed() >= std::time::Duration::from_secs(10) {
            self.configuration_finished = true;
            return false;
        }
        true
    }
    pub(crate) fn observe_configuration(
        &mut self,
        value: Option<&serde_json::Value>,
        unavailable: bool,
    ) {
        if let Some(value) = value {
            self.home_configuration = Some(parse_console_configuration(value));
            self.configuration_generation += 1;
            self.configuration_finished = true;
        } else if unavailable {
            self.configuration_finished = true;
        }
    }
    pub(crate) fn supplement_home_ice(
        &mut self,
        response: Option<Vec<IceCandidate>>,
        fallback: &[HostAddr],
        prefer_ipv6: bool,
    ) -> Option<Vec<IceCandidate>> {
        let mut candidates = Vec::new();
        for candidate in response.into_iter().flatten() {
            if is_end(&candidate) {
                if !self.pending_ends.contains(&candidate) {
                    self.pending_ends.push(candidate);
                }
                continue;
            }
            if let (Some(mid), Some(index), Some(ufrag)) = (
                &candidate.sdp_mid,
                candidate.sdp_m_line_index,
                candidate_ufrag(&candidate),
            ) {
                if self.remote_bindings.is_empty()
                    && self.remote_ufrags.iter().any(|current| current == ufrag)
                {
                    self.remote_bindings.push(RemoteTransportBinding {
                        mid: mid.clone(),
                        index,
                        ufrag: ufrag.to_owned(),
                    });
                }
            }
            let mut keyed = candidate.clone();
            if keyed.username_fragment.is_none() {
                keyed.username_fragment = self
                    .remote_bindings
                    .iter()
                    .find(|binding| Some(&binding.mid) == keyed.sdp_mid.as_ref())
                    .map(|binding| binding.ufrag.clone());
            }
            if let Some(key) = candidate_endpoint_key(&keyed) {
                self.remote_endpoint_keys.insert(key);
            }
            candidates.push(candidate);
        }
        for binding in &self.remote_bindings {
            let primary = self
                .home_configuration
                .as_ref()
                .map(|config| {
                    console_candidates(&config.addresses, binding, "session-configuration")
                })
                .unwrap_or_default();
            for candidate in primary.into_iter().chain(console_candidates(
                if self.configuration_finished {
                    fallback
                } else {
                    &[]
                },
                binding,
                "console-list",
            )) {
                if let Some(key) = candidate_endpoint_key(&candidate) {
                    if !self.remote_endpoint_keys.contains(&key) && self.supplement_keys.insert(key)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        let mut candidates = super::ice::IcePolicy::new(prefer_ipv6).normalize(&candidates);
        if self.configuration_finished {
            let mut ended = BTreeSet::new();
            for mut end in self.pending_ends.drain(..) {
                // 独立传输保留各自的结束标记；BUNDLE 成员回到主传输。
                let binding = self
                    .remote_bindings
                    .iter()
                    .find(|binding| {
                        end.sdp_mid.as_deref() == Some(binding.mid.as_str())
                            || end.sdp_m_line_index == Some(binding.index)
                    })
                    .or_else(|| self.remote_bindings.first());
                if let Some(binding) = binding {
                    end.sdp_mid = Some(binding.mid.clone());
                    end.sdp_m_line_index = Some(binding.index);
                    end.username_fragment = Some(binding.ufrag.clone());
                }
                if ended.insert((
                    end.sdp_mid.clone(),
                    end.sdp_m_line_index,
                    end.username_fragment.clone(),
                )) {
                    candidates.push(end);
                }
            }
        }
        (!candidates.is_empty()).then_some(candidates)
    }

    pub fn filter_ice(&mut self, response: Option<Vec<IceCandidate>>) -> Option<Vec<IceCandidate>> {
        let response = response?;
        let mut candidates = Vec::new();
        let mut stale_seen = false;
        for candidate in response.iter().filter(|candidate| !is_end(candidate)) {
            let belongs_to_current = if let Some(ufrag) =
                candidate_ufrag(candidate).filter(|_| !self.remote_ufrags.is_empty())
            {
                self.remote_ufrags.iter().any(|current| current == ufrag)
            } else {
                !self.baseline_ice.as_ref().is_some_and(|baseline| {
                    baseline.iter().any(|old| same_candidate(old, candidate))
                })
            };
            if belongs_to_current {
                candidates.push(candidate.clone());
            } else {
                stale_seen = true;
            }
        }
        // 旧快照的 EOC 无独立代际；跨轮时须与本次快照的有效候选一起准入。
        if !stale_seen && (self.baseline_ice.is_none() || !candidates.is_empty()) {
            candidates.extend(response.into_iter().filter(|candidate| {
                is_end(candidate)
                    && candidate_ufrag(candidate).is_none_or(|ufrag| {
                        self.remote_ufrags.is_empty()
                            || self.remote_ufrags.iter().any(|current| current == ufrag)
                    })
            }));
        }
        if candidates.is_empty() || self.last_polled_ice.as_ref() == Some(&candidates) {
            return None;
        }
        self.last_polled_ice = Some(candidates.clone());
        Some(candidates)
    }
}

pub(crate) fn media_ice_credentials(sdp: &str) -> Vec<String> {
    sdp_attributes(sdp, &["a=ice-ufrag:", "a=ice-pwd:"])
}

fn sdp_attributes(sdp: &str, prefixes: &[&str]) -> Vec<String> {
    sdp.lines()
        .filter_map(|line| {
            prefixes.iter().find_map(|prefix| {
                line.trim()
                    .strip_prefix(prefix)
                    .map(|value| value.to_owned())
            })
        })
        .collect()
}

fn candidate_ufrag(candidate: &IceCandidate) -> Option<&str> {
    candidate.username_fragment.as_deref().or_else(|| {
        let mut tokens = candidate.candidate.split_whitespace();
        while let Some(token) = tokens.next() {
            if token.eq_ignore_ascii_case("ufrag") {
                return tokens.next();
            }
        }
        None
    })
}

fn same_candidate(left: &IceCandidate, right: &IceCandidate) -> bool {
    left.candidate == right.candidate
        && left.sdp_mid == right.sdp_mid
        && left.sdp_m_line_index == right.sdp_m_line_index
        && candidate_ufrag(left) == candidate_ufrag(right)
}

fn is_end(candidate: &IceCandidate) -> bool {
    let raw = candidate.candidate.trim();
    raw.is_empty()
        || raw.eq_ignore_ascii_case("a=end-of-candidates")
        || raw.eq_ignore_ascii_case("end-of-candidates")
}

#[cfg(test)]
#[path = "state.test.rs"]
mod tests;
