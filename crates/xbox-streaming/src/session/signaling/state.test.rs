use super::*;

fn candidate(ufrag: Option<&str>) -> IceCandidate {
    IceCandidate {
        candidate: "candidate:1 1 UDP 1 192.0.2.1 5000 typ host".to_owned(),
        sdp_mid: Some("0".to_owned()),
        sdp_m_line_index: Some(0),
        username_fragment: ufrag.map(str::to_owned),
        ..Default::default()
    }
}
fn end() -> IceCandidate {
    IceCandidate {
        candidate: "a=end-of-candidates".to_owned(),
        ..Default::default()
    }
}
fn answer(ufrag: &str) -> AnswerPayload {
    AnswerPayload {
        sdp: format!("v=0\r\na=ice-ufrag:{ufrag}\r\n"),
        message_type: None,
    }
}
fn restarted() -> SessionSignalingState {
    let mut state = SessionSignalingState::default();
    state.apply_media_answer(&answer("old"));
    state.begin_media_ice(
        vec!["local-new".to_owned()],
        Some(vec![candidate(Some("old")), end()]),
    );
    state.apply_media_answer(&answer("new"));
    state
}

#[test]
fn rejects_old_credentials_and_end_marker_even_when_snapshot_changes() {
    let mut state = restarted();
    let mut old = candidate(Some("old"));
    old.candidate = "candidate:2 1 UDP 1 192.0.2.9 6000 typ host".to_owned();
    assert_eq!(state.filter_ice(Some(vec![old, end()])), None);
    assert_eq!(state.filter_ice(Some(vec![end()])), None);
    assert_eq!(state.filter_ice(None), None);
    assert_eq!(
        state.filter_ice(Some(vec![candidate(Some("old")), end()])),
        None
    );
}

#[test]
fn accepts_reused_address_with_current_ufrag_and_preserves_end_marker() {
    let mut state = restarted();
    let current = vec![candidate(Some("new")), end()];
    assert_eq!(
        state.filter_ice(Some(current.clone())),
        Some(current.clone())
    );
    assert_eq!(state.filter_ice(Some(current)), None);
    assert_eq!(
        state.filter_ice(Some(vec![candidate(Some("old")), end()])),
        None
    );
}

#[test]
fn withholds_end_marker_from_a_mixed_generation_snapshot() {
    let mut state = restarted();
    let current = candidate(Some("new"));
    assert_eq!(
        state.filter_ice(Some(vec![candidate(Some("old")), current.clone(), end()])),
        Some(vec![current.clone()])
    );
    assert_eq!(
        state.filter_ice(Some(vec![current.clone(), end()])),
        Some(vec![current, end()])
    );
}

#[test]
fn uses_candidate_extension_ufrag_when_json_field_is_absent() {
    let mut state = restarted();
    let mut current = candidate(None);
    current.candidate.push_str(" ufrag new");
    assert_eq!(
        state.filter_ice(Some(vec![current.clone(), end()])),
        Some(vec![current, end()])
    );
}

#[test]
fn filters_untagged_baseline_and_accepts_new_untagged_endpoint() {
    let old = candidate(None);
    let mut state = SessionSignalingState::default();
    state.begin_media_ice(vec![], Some(vec![old.clone(), end()]));
    state.apply_media_answer(&answer("new"));
    assert_eq!(state.filter_ice(Some(vec![old.clone(), end()])), None);
    let mut new = old;
    new.candidate = "candidate:1 1 UDP 1 192.0.2.1 5001 typ host".to_owned();
    assert_eq!(
        state.filter_ice(Some(vec![new.clone(), end()])),
        Some(vec![new, end()])
    );
}

#[test]
fn permits_same_candidates_when_fresh_answer_reuses_remote_credentials() {
    let mut state = SessionSignalingState::default();
    let candidates = vec![candidate(None), end()];
    state.apply_media_answer(&answer("same"));
    state.begin_media_ice(vec!["local-new".to_owned()], Some(candidates.clone()));
    state.apply_media_answer(&answer("same"));
    assert_eq!(state.filter_ice(Some(candidates.clone())), Some(candidates));
}

fn bound_state() -> SessionSignalingState {
    let mut state = SessionSignalingState::default();
    state.apply_media_answer(&AnswerPayload {
        sdp: "v=0\nm=video 9 UDP/TLS/RTP/SAVPF 96\na=mid:video\na=ice-ufrag:current\n".into(),
        message_type: None,
    });
    state
}
#[test]
fn config_can_supply_empty_ice_and_rebind_after_restart() {
    let mut state = bound_state();
    let config = serde_json::json!({"serverDetails":{"ipAddress":"192.0.2.1","port":5000}});
    state.observe_configuration(Some(&config), false);
    let first = state.supplement_home_ice(None, &[], false).unwrap();
    assert_eq!(first.len(), 2);
    assert!(state.supplement_home_ice(None, &[], false).is_none());
    state.apply_media_answer(&AnswerPayload {
        sdp: "v=0\nm=video 9 UDP/TLS/RTP/SAVPF 96\na=mid:video\na=ice-ufrag:current\n".into(),
        message_type: None,
    });
    assert!(state.supplement_home_ice(None, &[], false).is_none());
    state.begin_media_ice(vec!["new-local".into()], None);
    state.apply_media_answer(&AnswerPayload {
        sdp: "v=0\nm=video 9 UDP/TLS/RTP/SAVPF 96\na=mid:new-video\na=ice-ufrag:new\n".into(),
        message_type: None,
    });
    state.observe_configuration(Some(&config), false);
    assert!(state
        .supplement_home_ice(None, &[], false)
        .unwrap()
        .iter()
        .all(
            |candidate| candidate.username_fragment.as_deref() == Some("new")
                && candidate.sdp_mid.as_deref() == Some("new-video")
        ));
}
#[test]
fn remote_endpoint_wins_dedup_and_config_budget_releases_end_marker() {
    let mut state = bound_state();
    let mut remote = candidate(Some("current"));
    remote.sdp_mid = Some("video".into());
    state.observe_configuration(
        Some(&serde_json::json!({"serverDetails":{"ipAddress":"192.0.2.1","port":5000}})),
        false,
    );
    let candidates = state
        .supplement_home_ice(
            Some(vec![remote, end()]),
            &[HostAddr {
                ip: "192.0.2.1".into(),
                port: 5000,
            }],
            false,
        )
        .unwrap();
    assert_eq!(candidates.len(), 3);
    assert!(candidates[0].source.is_none());
    let mut state = bound_state();
    state
        .begin_configuration_poll(tokio::time::Instant::now() - std::time::Duration::from_secs(11));
    assert!(state
        .supplement_home_ice(Some(vec![end()]), &[], false)
        .is_some());
    assert!(!state.configuration_pending());
}

#[test]
fn pending_end_markers_keep_each_current_unbundled_transport() {
    let mut state = bound_state();
    state.apply_media_answer(&AnswerPayload {
        sdp: "v=0\nm=video 9 UDP/TLS/RTP/SAVPF 96\na=mid:video\na=ice-ufrag:current\nm=audio 9 UDP/TLS/RTP/SAVPF 111\na=mid:audio\na=ice-ufrag:voice\n".into(),
        message_type: None,
    });
    let mut video_end = end();
    video_end.sdp_mid = Some("video".into());
    video_end.sdp_m_line_index = Some(0);
    video_end.username_fragment = Some("current".into());
    let mut audio_end = end();
    audio_end.sdp_mid = Some("audio".into());
    audio_end.sdp_m_line_index = Some(1);
    audio_end.username_fragment = Some("voice".into());
    let filtered = state.filter_ice(Some(vec![
        video_end.clone(),
        audio_end.clone(),
        audio_end.clone(),
    ]));
    assert!(state.supplement_home_ice(filtered, &[], false).is_none());
    state.observe_configuration(None, true);
    let released = state.supplement_home_ice(None, &[], false).unwrap();
    assert_eq!(released, vec![video_end, audio_end]);
    assert!(state.supplement_home_ice(None, &[], false).is_none());
}

#[test]
fn rejects_end_marker_with_explicit_old_remote_credentials() {
    let mut state = bound_state();
    let mut old_end = end();
    old_end.username_fragment = Some("old".into());
    assert!(state.filter_ice(Some(vec![old_end])).is_none());
    assert!(state.filter_ice(Some(vec![end()])).is_some());
}
