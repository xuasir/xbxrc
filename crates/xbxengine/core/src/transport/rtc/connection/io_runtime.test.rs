
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn advertised_ip_priority_rejects_loopback() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(advertised_ip_priority(ip), None);
    }

    #[test]
    fn advertised_ip_priority_accepts_non_loopback_ipv4() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
        assert_eq!(advertised_ip_priority(ip), Some(2));
    }

    #[test]
    fn advertised_ip_priority_rejects_benchmark_ipv4() {
        let ip = IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1));
        assert_eq!(advertised_ip_priority(ip), None);
    }

    #[test]
    fn advertised_ip_priority_accepts_non_loopback_ipv6() {
        let ip = IpAddr::V6("240e:3a1:abcd::10".parse::<std::net::Ipv6Addr>().unwrap());
        assert_eq!(advertised_ip_priority(ip), Some(1));
    }

    #[test]
    fn advertised_ip_priority_rejects_unique_local_ipv6() {
        let ip = IpAddr::V6("fdfe:dcba:9876::1".parse::<std::net::Ipv6Addr>().unwrap());
        assert_eq!(advertised_ip_priority(ip), None);
    }

    #[test]
    fn resolve_local_addr_for_socket_keeps_bind_addr_when_family_differs() {
        let runtime = RtcIoRuntime {
            advertised_ips: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10))],
            ..Default::default()
        };
        let bind_addr: SocketAddr = "[::1]:7000".parse().unwrap();
        assert_eq!(runtime.resolve_local_addr_for_socket(bind_addr), bind_addr);
    }

    #[test]
    fn resolve_local_addr_for_socket_uses_advertised_ip_when_family_matches() {
        let runtime = RtcIoRuntime {
            advertised_ips: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10))],
            ..Default::default()
        };
        let bind_addr: SocketAddr = "0.0.0.0:7000".parse().unwrap();
        assert_eq!(
            runtime.resolve_local_addr_for_socket(bind_addr),
            "192.168.0.10:7000".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn local_host_endpoints_emit_ipv4_and_ipv6_without_mixing_ports() {
        let runtime = RtcIoRuntime {
            local_addr_v4: Some("0.0.0.0:7000".parse().unwrap()),
            local_addr_v6: Some("[::]:8000".parse().unwrap()),
            advertised_ips: vec![
                IpAddr::V6(
                    "2408:8352:a12:20e0::e6a"
                        .parse::<std::net::Ipv6Addr>()
                        .unwrap(),
                ),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 122)),
            ],
            ..Default::default()
        };
        let endpoints = runtime.local_host_endpoints();
        assert_eq!(endpoints.len(), 2);
        assert_eq!(
            endpoints[0],
            "[2408:8352:a12:20e0::e6a]:8000"
                .parse::<SocketAddr>()
                .unwrap()
        );
        assert_eq!(
            endpoints[1],
            "10.0.0.122:7000".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn prefer_ipv6_only_changes_host_endpoint_order() {
        let mut runtime = RtcIoRuntime {
            local_addr_v4: Some("0.0.0.0:7000".parse().unwrap()),
            local_addr_v6: Some("[::]:8000".parse().unwrap()),
            advertised_ips: vec![
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 122)),
                IpAddr::V6(
                    "2408:8352:a12:20e0::e6a"
                        .parse::<std::net::Ipv6Addr>()
                        .unwrap(),
                ),
            ],
            ..Default::default()
        };
        runtime.set_prefer_ipv6(true);
        let endpoints = runtime.local_host_endpoints();
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints[0].is_ipv6());
        assert!(endpoints[1].is_ipv4());
    }

    #[test]
    fn choose_preferred_advertised_ip_prefers_private_over_benchmark() {
        let chosen = choose_preferred_advertised_ip(vec![
            IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
        ]);
        assert_eq!(chosen, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
    }

    #[test]
    fn choose_preferred_advertised_ip_prefers_private_ipv4_over_global_ipv6() {
        let chosen = choose_preferred_advertised_ip(vec![
            IpAddr::V6(
                "2408:8352:a12:20e0::e6a"
                    .parse::<std::net::Ipv6Addr>()
                    .unwrap(),
            ),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 122)),
        ]);

        assert_eq!(chosen, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 122))));
    }

    #[test]
    fn collect_srflx_probe_urls_keeps_only_udp_stun_like_urls() {
        let session = XbxEngineSessionDto {
            session_id: "test-session".to_string(),
            target_type: xbxengine_protocol::XbxEngineTargetTypeDto::Home,
            turn_server: Some(xbxengine_protocol::XbxEngineTurnServerDto {
                url: "turn:relay.example.com:3478?transport=udp".to_string(),
                username: "u".to_string(),
                credential: "p".to_string(),
            }),
        };

        let urls = collect_srflx_probe_urls(&session);

        assert!(urls.iter().all(|reference| {
            reference.raw_url.starts_with("stun:") || reference.raw_url.starts_with("turn:")
        }));
        assert!(urls
            .iter()
            .all(|reference| { reference.raw_url.contains(":3478") }));
    }

    #[test]
    fn resolve_relay_related_addr_uses_advertised_ip_for_unspecified_base_addr() {
        let related = resolve_relay_related_addr(
            "0.0.0.0:45678".parse().unwrap(),
            &[IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10))],
        );
        assert_eq!(related, "192.168.0.10:45678".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn resolve_relay_related_addr_prefers_ipv6_for_unspecified_ipv6_base_addr() {
        let related = resolve_relay_related_addr(
            "[::]:45678".parse().unwrap(),
            &[
                IpAddr::V4(Ipv4Addr::new(192, 168, 0, 10)),
                IpAddr::V6(
                    "2408:8352:a12:20e0::e6a"
                        .parse::<std::net::Ipv6Addr>()
                        .unwrap(),
                ),
            ],
        );
        assert_eq!(
            related,
            "[2408:8352:a12:20e0::e6a]:45678"
                .parse::<SocketAddr>()
                .unwrap()
        );
    }

    #[test]
    fn parse_ice_server_url_accepts_rfc7064_style_url() {
        let parsed = parse_ice_server_url("stun:stun.example.com:3478").unwrap();
        assert_eq!(parsed.host, "stun.example.com");
        assert_eq!(parsed.port, 3478);
    }

    #[test]
    fn non_fatal_send_error_detection_covers_no_route_family() {
        assert!(is_non_fatal_send_error(&std::io::Error::new(
            ErrorKind::HostUnreachable,
            "no route to host",
        )));
        assert!(is_non_fatal_send_error(&std::io::Error::new(
            ErrorKind::NetworkUnreachable,
            "network unreachable",
        )));
        assert!(is_non_fatal_send_error(&std::io::Error::new(
            ErrorKind::AddrNotAvailable,
            "address not available",
        )));
        assert!(!is_non_fatal_send_error(&std::io::Error::new(
            ErrorKind::PermissionDenied,
            "permission denied",
        )));
    }

    #[test]
    fn non_fatal_send_drop_window_only_fails_after_grace_and_threshold() {
        let mut runtime = RtcIoRuntime::default();
        let now = Instant::now();
        let peers = HashSet::from(["10.0.0.2:3478".parse::<SocketAddr>().unwrap()]);
        runtime
            .update_non_fatal_send_drop_window(false, &peers, 3, now)
            .unwrap();
        assert!(runtime
            .update_non_fatal_send_drop_window(
                false,
                &peers,
                3,
                now + RTC_NON_FATAL_SEND_DROP_ERROR_GRACE + Duration::from_millis(1),
            )
            .is_err());
    }

    #[test]
    fn non_fatal_send_drop_window_resets_when_network_progress_observed() {
        let mut runtime = RtcIoRuntime::default();
        let now = Instant::now();
        let peers = HashSet::from(["10.0.0.2:3478".parse::<SocketAddr>().unwrap()]);
        runtime
            .update_non_fatal_send_drop_window(false, &peers, 5, now)
            .unwrap();
        runtime
            .update_non_fatal_send_drop_window(true, &HashSet::new(), 0, now)
            .unwrap();
        assert!(runtime.non_fatal_send_drop_window.started_at.is_none());
        assert_eq!(runtime.non_fatal_send_drop_window.drop_count, 0);
        assert!(runtime.non_fatal_send_drop_window.peers.is_empty());
    }
