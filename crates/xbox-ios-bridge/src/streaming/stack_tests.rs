use super::*;
use std::future::Future;
use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, OnceLock};
use std::time::Instant;
use uniffi::{Handle, Lift, RustCallStatus, RustCallStatusCode, RustFuturePoll};

// 实机 Swift 工作线程约 544 KiB；为 UniFFI 按值包装及下游轮询保留空间。
const EXPORTED_FUTURE_BUDGET: usize = 8 * 1024;
const DEVICE_THREAD_STACK: usize = 544 * 1024;

fn future_size<F: Future>(
    name: &'static str,
    _: impl FnOnce(&'static XboxStreamSession) -> F,
) -> (&'static str, usize) {
    (name, std::mem::size_of::<F>())
}

#[test]
fn exported_async_futures_stay_within_stack_budget() {
    let sizes = [
        future_size("start", |s| s.start()),
        future_size("pending_auth_update", |s| s.pending_auth_update()),
        future_size("acknowledge_auth_update", |s| s.acknowledge_auth_update(1)),
        future_size("synchronize_auth_session", |s| {
            s.synchronize_auth_session(
                1,
                crate::AuthSession {
                    refresh_token: String::new(),
                    seed_json: String::new(),
                    web_token_json: String::new(),
                    app_level: 0,
                },
            )
        }),
        future_size("exchange_offer", |s| s.exchange_offer(1, String::new())),
        future_size("exchange_media_offer", |s| {
            s.exchange_media_offer(1, String::new(), String::new())
        }),
        future_size("supervise_runtime", |s| {
            s.supervise_runtime(1, String::new())
        }),
        future_size("validate_resume", |s| s.validate_resume(1)),
        future_size("submit_ice", |s| s.submit_ice(1, Vec::new())),
        future_size("next_remote_ice_batch", |s| s.next_remote_ice_batch(1)),
        future_size("cancel", |s| s.cancel()),
        future_size("close", |s| s.close()),
        future_size("mark_connected", |s| s.mark_connected(1)),
        future_size("send_text", |s| s.send_text(1, String::new())),
        future_size("power_off_console", |s| s.power_off_console(1)),
    ];
    for (name, size) in sizes {
        eprintln!("exported_future_size {name}={size}");
    }
    let oversized = sizes
        .into_iter()
        .filter(|(_, size)| *size > EXPORTED_FUTURE_BUDGET)
        .collect::<Vec<_>>();
    assert!(
        oversized.is_empty(),
        "exported async future exceeds {EXPORTED_FUTURE_BUDGET}-byte stack budget: {oversized:?}"
    );
}

fn scoped_future_size<F: Future>(
    name: &'static str,
    _: impl FnOnce(&'static XboxStreamSession, &'static SessionStartupScope) -> F,
) -> (&'static str, usize) {
    (name, std::mem::size_of::<F>())
}

#[test]
fn boxed_startup_stages_preserve_stack_headroom() {
    let sizes = [
        scoped_future_size("prepare_start_plan", |s, scope| s.prepare_start_plan(scope)),
        scoped_future_size("home_context", |s, scope| {
            home_context::resolve_home_context(
                s.access.clone(),
                &s.credentials,
                &s.target_id,
                &s.settings,
                scope,
            )
        }),
        scoped_future_size("session_execution", |s, scope| {
            s.flow.start_session_execution_with_scope(
                s.plan_snapshot().unwrap(),
                |_| (),
                |_| (),
                None::<&IosStartupObserver>,
                scope,
            )
        }),
    ];
    for (name, size) in sizes {
        eprintln!("startup_stage_future_size {name}={size}");
    }
    assert!(
        sizes.iter().all(|(_, size)| *size <= 8 * 1024),
        "startup stage future exceeds 8 KiB size budget: {sizes:?}"
    );
}

#[test]
fn credential_operations_preserve_stack_headroom() {
    let sizes = [
        future_size("credential_access", |s| {
            s.credentials.access("home", None, None)
        }),
        future_size("credential_renewal", |s| {
            crate::cloud_access::renew_bound_stream_access(&s.access)
        }),
        scoped_future_size("scoped_credential_access", |s, scope| {
            scope.run(
                "credentialProbe",
                Some(Duration::from_secs(5)),
                async move {
                    s.credentials
                        .access("home", None, None)
                        .await
                        .map(|access| access.owner_generation)
                        .map_err(SessionFlowError::from)
                },
            )
        }),
    ];
    for (name, size) in sizes {
        eprintln!("credential_future_size {name}={size}");
    }
    assert!(
        sizes.iter().all(|(_, size)| *size <= 8 * 1024),
        "credential future exceeds 8 KiB stack budget: {sizes:?}"
    );
}

#[test]
fn home_and_cloud_provider_credentials_poll_on_device_stack() {
    for target in [Target::Home, Target::Cloud] {
        for expired in [false, true] {
            on_device_stack(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async move {
                    let access = super::tests::test_access_for_account(
                        target,
                        &uuid::Uuid::new_v4().to_string(),
                    );
                    let (handle, mut access) =
                        crate::cloud_access::register_test_stream_access(access);
                    let generation = access.owner_generation;
                    if expired {
                        access.expires_at_ms = 0;
                    }
                    let credentials = Arc::new(SessionCredentials::new(handle.clone(), access));
                    let weak = Arc::downgrade(&credentials);
                    let provider = IosSessionFlowProvider {
                        credentials,
                        target,
                        projection: Arc::new(StdMutex::new(SessionFlowProjection::default())),
                    };
                    let scope = SessionStartupScope::default();
                    let result = scope
                        .run(
                            "providerToken",
                            Some(Duration::from_secs(2)),
                            provider.get_streaming_token(target.as_str()),
                        )
                        .await;
                    if expired {
                        assert_eq!(result.unwrap_err().status, Some(401));
                    } else {
                        let token = result.unwrap();
                        assert_eq!(token["gsToken"], "gs-token");
                        assert_eq!(token["_authGeneration"], generation);
                        let rejected = provider
                            .recover_streaming_token(target.as_str(), "gs-token", Some(generation))
                            .await
                            .unwrap_err();
                        assert_eq!(rejected.to_status_code(), Some(401));
                    }
                    // 缺失 seed 的真实续期分支返回 typed 401，安全存储更新保持空值。
                    assert!(provider.credentials.pending_update().await.is_none());
                    drop(provider);
                    assert!(weak.upgrade().is_none());
                    crate::cloud_access::release_stream_access(handle).unwrap();
                });
            });
        }
    }
}

type PollSenders = std::collections::HashMap<u64, mpsc::Sender<RustFuturePoll>>;
static POLL_SENDERS: OnceLock<StdMutex<PollSenders>> = OnceLock::new();
static NEXT_CALLBACK: AtomicU64 = AtomicU64::new(1);

extern "C" fn on_future_poll(data: u64, poll: RustFuturePoll) {
    let sender = POLL_SENDERS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .get(&data)
        .cloned();
    if let Some(sender) = sender {
        let _ = sender.send(poll);
    }
}

struct StartFuture {
    handle: Handle,
    callback_id: u64,
    updates: mpsc::Receiver<RustFuturePoll>,
}

enum StartOutcome {
    Ready(XboxPreparedSignaling),
    Error(XboxStreamingError),
    Cancelled,
}

impl StartFuture {
    fn new(session: Arc<XboxStreamSession>) -> Self {
        let (sender, updates) = mpsc::channel();
        let callback_id = NEXT_CALLBACK.fetch_add(1, Ordering::Relaxed);
        POLL_SENDERS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(callback_id, sender);
        Self {
            handle: uniffi_xbox_ios_bridge_fn_method_xboxstreamsession_start(Handle::from_arc(
                session,
            )),
            callback_id,
            updates,
        }
    }

    fn poll(&self) {
        // handle 归本夹具持有，poll/cancel/complete/free 顺序与 Swift bindings 一致。
        unsafe {
            crate::ffi_xbox_ios_bridge_rust_future_poll_rust_buffer(
                self.handle.clone(),
                on_future_poll,
                self.callback_id,
            );
        }
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        self.poll();
        loop {
            match self
                .updates
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("FFI continuation before deadline")
            {
                RustFuturePoll::Ready => return,
                RustFuturePoll::Wake => self.poll(),
            }
        }
    }

    fn cancel(&self) {
        unsafe {
            crate::ffi_xbox_ios_bridge_rust_future_cancel_rust_buffer(self.handle.clone());
        }
    }

    fn complete(&self) -> StartOutcome {
        let mut status = RustCallStatus::default();
        let buffer = unsafe {
            crate::ffi_xbox_ios_bridge_rust_future_complete_rust_buffer(
                self.handle.clone(),
                &mut status,
            )
        };
        let error_buffer = ManuallyDrop::into_inner(status.error_buf);
        match status.code {
            RustCallStatusCode::Success => {
                error_buffer.destroy();
                StartOutcome::Ready(
                    <XboxPreparedSignaling as Lift<crate::UniFfiTag>>::try_lift(buffer)
                        .expect("prepared signaling record"),
                )
            }
            RustCallStatusCode::Error => {
                buffer.destroy();
                StartOutcome::Error(
                    <XboxStreamingError as Lift<crate::UniFfiTag>>::try_lift(error_buffer)
                        .expect("streaming error record"),
                )
            }
            RustCallStatusCode::Cancelled => {
                buffer.destroy();
                error_buffer.destroy();
                StartOutcome::Cancelled
            }
            RustCallStatusCode::UnexpectedError => {
                buffer.destroy();
                error_buffer.destroy();
                panic!("unexpected UniFFI failure");
            }
        }
    }
}

impl Drop for StartFuture {
    fn drop(&mut self) {
        unsafe {
            crate::ffi_xbox_ios_bridge_rust_future_free_rust_buffer(self.handle.clone());
        }
        POLL_SENDERS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(&self.callback_id);
    }
}

fn on_device_stack(work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name("ios-uniffi-small-stack".into())
        .stack_size(DEVICE_THREAD_STACK)
        .spawn(work)
        .unwrap()
        .join()
        .expect("544 KiB bridge worker completed");
}

fn fixture_session(target: Target) -> Arc<XboxStreamSession> {
    build_stream_session(
        "invalid-stack-fixture-handle".into(),
        super::tests::test_access_for_account(target, &uuid::Uuid::new_v4().to_string()),
        "1234abcd".into(),
        XboxStreamSettings::default(),
    )
    .unwrap()
}

#[test]
fn ffi_start_returns_credential_error_and_frees_on_device_stack() {
    for target in [Target::Cloud, Target::Home] {
        on_device_stack(move || {
            let session = fixture_session(target);
            let weak = Arc::downgrade(&session);
            let future = StartFuture::new(Arc::clone(&session));
            future.wait_ready();
            assert!(matches!(
                future.complete(),
                StartOutcome::Error(XboxStreamingError::Http(401, _))
            ));
            assert_eq!(
                session.state.try_lock().unwrap().phase,
                SessionPhase::Failed
            );
            drop(future);
            drop(session);
            assert!(
                weak.upgrade().is_none(),
                "future released session ownership"
            );
        });
    }
}

#[test]
fn ffi_start_cancels_a_pending_poll_and_frees_on_device_stack() {
    for target in [Target::Cloud, Target::Home] {
        on_device_stack(move || {
            let session = fixture_session(target);
            let weak = Arc::downgrade(&session);
            let state = session.state.try_lock().unwrap();
            let future = StartFuture::new(Arc::clone(&session));
            future.poll();
            assert!(matches!(
                future.updates.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            future.cancel();
            assert!(matches!(
                future.updates.recv_timeout(Duration::from_secs(2)).unwrap(),
                RustFuturePoll::Ready
            ));
            assert!(matches!(future.complete(), StartOutcome::Cancelled));
            assert_eq!(state.phase, SessionPhase::Idle);
            drop(future);
            drop(state);
            drop(session);
            assert!(
                weak.upgrade().is_none(),
                "cancelled future released ownership"
            );
        });
    }
}

#[test]
fn ffi_start_free_drops_a_pending_operation_on_device_stack() {
    for target in [Target::Cloud, Target::Home] {
        on_device_stack(move || {
            let session = fixture_session(target);
            let weak = Arc::downgrade(&session);
            let state = session.state.try_lock().unwrap();
            let future = StartFuture::new(Arc::clone(&session));
            future.poll();
            assert!(matches!(
                future.updates.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            let callback_id = future.callback_id;
            drop(future);
            assert!(!POLL_SENDERS
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .contains_key(&callback_id));
            assert_eq!(state.phase, SessionPhase::Idle);
            drop(state);
            drop(session);
            assert!(
                weak.upgrade().is_none(),
                "pending operation released ownership"
            );
        });
    }
}

struct SessionServer {
    address: std::net::SocketAddr,
    requests: Arc<StdMutex<Vec<String>>>,
    stopping: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl SessionServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_requests = Arc::clone(&requests);
        let worker_stopping = Arc::clone(&stopping);
        let worker = std::thread::spawn(move || {
            while let Ok((mut socket, _)) = listener.accept() {
                if worker_stopping.load(Ordering::Acquire) {
                    break;
                }
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0u8; 2048];
                    let count = socket.read(&mut chunk).unwrap_or(0);
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let body_len = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if bytes.len() < end + 4 + body_len {
                            continue;
                        }
                        let request = headers.lines().next().unwrap().to_owned();
                        eprintln!("stack_fixture_request {request}");
                        let (status, body) = if request.starts_with("POST /v5/sessions/cloud/play ")
                        {
                            (
                                200,
                                json!({"sessionPath":"/v5/sessions/cloud/stack-fixture"}),
                            )
                        } else if request.starts_with("POST /v5/sessions/home/play ") {
                            (
                                200,
                                json!({"sessionPath":"/v5/sessions/home/stack-fixture"}),
                            )
                        } else if request.starts_with("GET /v6/servers/home?mr=50 ") {
                            (200, json!({"results":[]}))
                        } else if request.contains("/stack-fixture/state ") {
                            (200, json!({"state":"Provisioned"}))
                        } else if request.starts_with("DELETE /v5/sessions/cloud/stack-fixture ")
                            || request.starts_with("DELETE /v5/sessions/home/stack-fixture ")
                        {
                            (200, json!({}))
                        } else if request.starts_with("CONNECT ") {
                            // Home 的真实 Smartglass 查询收到临时故障后消费已缓存主机事实。
                            (502, json!({"error":"fixture blocks upstream requests"}))
                        } else {
                            (404, json!({"error":"unexpected fixture request"}))
                        };
                        worker_requests.lock().unwrap().push(request);
                        let body = body.to_string();
                        let response = format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(response.as_bytes());
                        break;
                    }
                }
            }
        });
        Self {
            address,
            requests,
            stopping,
            worker: Some(worker),
        }
    }
}

impl Drop for SessionServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn ffi_cloud_start_drives_real_session_flow_on_device_stack() {
    // 本地 HTTP 夹具在独立进程排除代理，保持并行测试的进程环境稳定。
    if std::env::var_os("XBXRC_STACK_FIX_HTTP_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "streaming::stack_tests::ffi_cloud_start_drives_real_session_flow_on_device_stack",
                "--nocapture",
            ])
            .env("XBXRC_STACK_FIX_HTTP_CHILD", "1")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .status()
            .expect("isolated local HTTP regression");
        assert!(status.success(), "isolated FFI startup regression passed");
        return;
    }
    on_device_stack(|| {
        let server = SessionServer::new();
        let mut access =
            super::tests::test_access_for_account(Target::Cloud, &uuid::Uuid::new_v4().to_string());
        access.host = server.address.to_string();
        access.session_access.regions[0].base_uri = format!("http://{}", server.address);
        let (handle, access) = crate::cloud_access::register_test_stream_access(access);
        let settings = XboxStreamSettings {
            cloud_resolution: 1080,
            ..Default::default()
        };
        let session =
            build_stream_session(handle.clone(), access, "1234abcd".into(), settings).unwrap();
        let weak = Arc::downgrade(&session);
        let future = StartFuture::new(Arc::clone(&session));
        future.wait_ready();
        let outcome = future.complete();
        drop(future);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(session.close()).unwrap();
        crate::cloud_access::release_stream_access(handle).unwrap();
        match outcome {
            StartOutcome::Ready(prepared) => {
                assert_eq!(prepared.generation, 1);
                assert_eq!(prepared.web_rtc_plan.target_video_width, 1920);
            }
            StartOutcome::Error(error) => panic!("fixture startup failed: {error:?}"),
            StartOutcome::Cancelled => panic!("fixture startup cancelled"),
        }
        drop(session);
        assert!(
            weak.upgrade().is_none(),
            "completed future released ownership"
        );
        let requests = server.requests.lock().unwrap();
        assert_eq!(
            requests.iter().filter(|r| r.starts_with("POST ")).count(),
            1
        );
        assert!(requests.iter().any(|r| r.contains("/stack-fixture/state ")));
        assert_eq!(
            requests.iter().filter(|r| r.starts_with("DELETE ")).count(),
            1
        );
    });
}

#[test]
fn ffi_home_start_drives_real_session_flow_on_device_stack() {
    const CHILD_ENV: &str = "XBXRC_HOME_STACK_CHILD";
    const ADDRESS_ENV: &str = "XBXRC_HOME_STACK_SERVER";
    if std::env::var_os(CHILD_ENV).is_none() {
        let server = SessionServer::new();
        let address = server.address.to_string();
        let proxy = format!("http://{address}");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "streaming::stack_tests::ffi_home_start_drives_real_session_flow_on_device_stack",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env(ADDRESS_ENV, &address)
            .env("HTTPS_PROXY", &proxy)
            .env("https_proxy", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("http_proxy", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("all_proxy", &proxy)
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .status()
            .expect("isolated Home HTTP regression");
        assert!(status.success(), "isolated Home FFI startup passed");
        let requests = server.requests.lock().unwrap();
        assert!(requests.iter().any(|r| r.starts_with("CONNECT ")));
        assert!(requests
            .iter()
            .any(|r| r.starts_with("GET /v6/servers/home?mr=50 ")));
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /v5/sessions/home/play "))
                .count(),
            1
        );
        assert!(requests.iter().any(|r| r.contains("/stack-fixture/state ")));
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("DELETE /v5/sessions/home/stack-fixture "))
                .count(),
            1
        );
        return;
    }
    // 子进程仅连本地服务：HTTPS 上游由夹具拒绝，控制 HTTP 沿 loopback 实际执行。
    on_device_stack(|| {
        let address = std::env::var(ADDRESS_ENV).unwrap();
        let mut access =
            super::tests::test_access_for_account(Target::Home, &uuid::Uuid::new_v4().to_string());
        access.host = address.clone();
        access.session_access.regions[0].base_uri = format!("http://{address}");
        let (handle, access) = crate::cloud_access::register_test_stream_access(access);
        let settings = XboxStreamSettings {
            home_resolution: 1080,
            ..Default::default()
        };
        let session =
            build_stream_session(handle.clone(), access, "1234abcd".into(), settings).unwrap();
        let weak = Arc::downgrade(&session);
        let future = StartFuture::new(Arc::clone(&session));
        future.wait_ready();
        let outcome = future.complete();
        drop(future);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(session.close()).unwrap();
        crate::cloud_access::release_stream_access(handle).unwrap();
        match outcome {
            StartOutcome::Ready(prepared) => {
                assert_eq!(prepared.generation, 1);
                assert_eq!(prepared.web_rtc_plan.target_video_width, 1920);
            }
            StartOutcome::Error(error) => panic!("Home fixture startup failed: {error:?}"),
            StartOutcome::Cancelled => panic!("Home fixture startup cancelled"),
        }
        drop(session);
        assert!(
            weak.upgrade().is_none(),
            "Home FFI future released ownership"
        );
    });
    // 主动续期继续进入真实 AuthFlow；夹具拒绝 HTTPS 上游并透传临时认证错误。
    for target in [Target::Cloud, Target::Home] {
        on_device_stack(move || {
            let address = std::env::var(ADDRESS_ENV).unwrap();
            let mut access =
                super::tests::test_access_for_account(target, &uuid::Uuid::new_v4().to_string());
            access.host = address.clone();
            access.session_access.regions[0].base_uri = format!("http://{address}");
            access.auth_seed = Some(xbox_auth_flow::AuthFlowSeed {
                private_jwk: json!({}),
            });
            let (handle, mut access) = crate::cloud_access::register_test_stream_access(access);
            access.expires_at_ms = 0;
            let session = build_stream_session(
                handle.clone(),
                access,
                "1234abcd".into(),
                XboxStreamSettings::default(),
            )
            .unwrap();
            let weak = Arc::downgrade(&session);
            let future = StartFuture::new(Arc::clone(&session));
            future.wait_ready();
            assert!(matches!(
                future.complete(),
                StartOutcome::Error(XboxStreamingError::Http(503, _))
            ));
            assert_eq!(
                session.state.try_lock().unwrap().phase,
                SessionPhase::Failed
            );
            drop(future);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            assert!(runtime
                .block_on(session.credentials.pending_update())
                .is_none());
            runtime.block_on(session.close()).unwrap();
            crate::cloud_access::release_stream_access(handle).unwrap();
            drop(session);
            assert!(
                weak.upgrade().is_none(),
                "renewal future released ownership"
            );
        });
    }
}
