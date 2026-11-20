use super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll, Waker};

const DEVICE_STACK_BYTES: usize = 544 * 1024;
const OPERATION_BYTES: usize = 64 * 1024;

struct LargeOperation {
    payload: [u8; OPERATION_BYTES],
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    ready: bool,
}

impl Future for LargeOperation {
    type Output = Result<u8, SessionFlowError>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        std::hint::black_box(&self.payload);
        self.polls.fetch_add(1, Ordering::Relaxed);
        if self.ready {
            Poll::Ready(Ok(7))
        } else {
            Poll::Pending
        }
    }
}

impl Drop for LargeOperation {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

fn large_operation(
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    ready: bool,
) -> LargeOperation {
    LargeOperation {
        payload: [7; OPERATION_BYTES],
        polls,
        drops,
        ready,
    }
}

fn nested_operation<'a>(
    scope: &'a SessionStartupScope,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    ready: bool,
    timeout: Option<Duration>,
) -> impl Future<Output = Result<u8, SessionFlowError>> + 'a {
    scope.run(
        "outer",
        timeout,
        scope.run("inner", None, large_operation(polls, drops, ready)),
    )
}

fn future_size<F: Future>(_: impl FnOnce() -> F) -> usize {
    std::mem::size_of::<F>()
}

#[test]
fn operation_wrappers_bound_large_future_layout() {
    let scope = SessionStartupScope::default();
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let sizes = [
        (
            "cancel",
            future_size(|| {
                scope.inner.cancelled.run(
                    "cancel",
                    None,
                    large_operation(Arc::clone(&polls), Arc::clone(&drops), false),
                )
            }),
        ),
        (
            "scope",
            future_size(|| {
                scope.run(
                    "scope",
                    None,
                    large_operation(Arc::clone(&polls), Arc::clone(&drops), false),
                )
            }),
        ),
        (
            "nested",
            future_size(|| {
                nested_operation(
                    &scope,
                    Arc::clone(&polls),
                    Arc::clone(&drops),
                    false,
                    Some(Duration::from_secs(1)),
                )
            }),
        ),
    ];
    for (name, bytes) in sizes {
        eprintln!("operation_wrapper_future_size {name}={bytes}");
    }
    assert!(
        sizes.iter().all(|(_, bytes)| *bytes <= 4096),
        "64 KiB operation requires bounded wrappers: {sizes:?}"
    );
}

#[test]
fn large_nested_operations_poll_cancel_timeout_and_drop_on_small_stack() {
    for case in [
        "ready",
        "timeout",
        "cancel",
        "already_cancelled",
        "drop_before_poll",
        "drop_pending",
    ] {
        std::thread::Builder::new()
            .name(format!("operation-small-stack-{case}"))
            .stack_size(DEVICE_STACK_BYTES)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let scope = SessionStartupScope::default();
                let polls = Arc::new(AtomicUsize::new(0));
                let drops = Arc::new(AtomicUsize::new(0));
                let timeout = (case == "timeout").then_some(Duration::from_millis(5));
                let future = nested_operation(
                    &scope,
                    Arc::clone(&polls),
                    Arc::clone(&drops),
                    case == "ready",
                    timeout,
                );
                if case == "already_cancelled" {
                    scope.cancel();
                }
                if case == "cancel" {
                    let cancelling = scope.clone();
                    runtime.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        cancelling.cancel();
                    });
                }
                if case == "drop_before_poll" {
                    drop(future);
                    assert!(!scope.is_cancelled());
                } else if case == "drop_pending" {
                    let _entered = runtime.enter();
                    let mut future = Box::pin(future);
                    assert!(future
                        .as_mut()
                        .poll(&mut Context::from_waker(Waker::noop()))
                        .is_pending());
                    drop(future);
                    assert!(!scope.is_cancelled());
                } else {
                    let result = runtime.block_on(future);
                    match case {
                        "ready" => {
                            assert_eq!(result.unwrap(), 7);
                            assert!(!scope.is_cancelled());
                        }
                        "timeout" => {
                            assert_eq!(result.unwrap_err().message, "operationTimeout:stage=outer");
                            assert!(scope.is_cancelled());
                        }
                        "cancel" | "already_cancelled" => {
                            assert_eq!(
                                result.unwrap_err().message,
                                "operationCancelled:stage=outer"
                            );
                            assert!(scope.is_cancelled());
                        }
                        _ => unreachable!(),
                    }
                }
                assert_eq!(drops.load(Ordering::Relaxed), 1, "{case}");
                if case == "already_cancelled" || case == "drop_before_poll" {
                    assert_eq!(polls.load(Ordering::Relaxed), 0);
                } else {
                    assert!(polls.load(Ordering::Relaxed) > 0, "{case}");
                }
            })
            .unwrap()
            .join()
            .expect("544 KiB operation worker completed");
    }
}
