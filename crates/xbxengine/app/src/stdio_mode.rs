use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use xbxengine::{NoopXbxEngineInputBackend, XbxEngineRuntimeError};
use xbxengine_app::{
    SharedStateXbxEngineWindowHost, SharedXbxEngineWindowState, XbxEngineApp,
    XbxEngineAppHostBridge,
};
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineControlResponseDto, XbxEngineHostRequestDto,
    XbxEngineHostResponseDto, XbxEngineIncomingMessageDto, XbxEngineOutgoingMessageDto,
};

type HostResponseResult = Result<XbxEngineHostResponseDto, String>;

enum WorkerCommand {
    ControlRequest {
        request_id: String,
        command: XbxEngineControlCommandDto,
    },
    Shutdown,
}

#[derive(Default)]
struct SharedOutgoingMessages {
    messages: Mutex<Vec<XbxEngineOutgoingMessageDto>>,
}

impl SharedOutgoingMessages {
    fn push(&self, message: XbxEngineOutgoingMessageDto) {
        self.messages
            .lock()
            .expect("lock stdio outgoing messages")
            .push(message);
    }

    fn drain(&self) -> Vec<XbxEngineOutgoingMessageDto> {
        self.messages
            .lock()
            .expect("lock stdio outgoing messages")
            .drain(..)
            .collect()
    }
}

struct StdioHostBridge {
    outgoing_messages: Arc<SharedOutgoingMessages>,
    pending_host_responses: Arc<Mutex<HashMap<String, mpsc::Sender<HostResponseResult>>>>,
    next_request_id: Arc<AtomicU64>,
    shutdown_flag: Arc<AtomicBool>,
}

impl XbxEngineAppHostBridge for StdioHostBridge {
    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        let request_id = format!(
            "host-{}",
            self.next_request_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (sender, receiver) = mpsc::channel::<HostResponseResult>();
        self.pending_host_responses
            .lock()
            .expect("lock pending host responses")
            .insert(request_id.clone(), sender);
        self.outgoing_messages
            .push(XbxEngineOutgoingMessageDto::HostRequest {
                request_id: request_id.clone(),
                request,
            });

        loop {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                self.pending_host_responses
                    .lock()
                    .expect("lock pending host responses")
                    .remove(&request_id);
                return Err(XbxEngineRuntimeError::new("xbxengineAppStdioBridgeClosed"));
            }

            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(message)) => return Err(XbxEngineRuntimeError::new(message)),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(XbxEngineRuntimeError::new(
                        "xbxengineAppHostResponseChannelClosed",
                    ))
                }
            }
        }
    }
}

pub struct StdioSidecarMode {
    sender: mpsc::Sender<WorkerCommand>,
    shutdown_flag: Arc<AtomicBool>,
    stdin_handle: Option<JoinHandle<()>>,
    stdout_handle: Option<JoinHandle<()>>,
    worker_handle: Option<JoinHandle<()>>,
}

impl StdioSidecarMode {
    pub fn spawn(
        window_state: SharedXbxEngineWindowState,
    ) -> Result<(Arc<Mutex<XbxEngineApp>>, Self), String> {
        let (sender, receiver) = mpsc::channel::<WorkerCommand>();
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let outgoing_messages = Arc::new(SharedOutgoingMessages::default());
        let pending_host_responses = Arc::new(Mutex::new(HashMap::<
            String,
            mpsc::Sender<HostResponseResult>,
        >::new()));
        let next_request_id = Arc::new(AtomicU64::new(0));

        let app = Arc::new(Mutex::new(XbxEngineApp::with_runtime_hosts(
            Box::new(StdioHostBridge {
                outgoing_messages: outgoing_messages.clone(),
                pending_host_responses: pending_host_responses.clone(),
                next_request_id: next_request_id.clone(),
                shutdown_flag: shutdown_flag.clone(),
            }),
            Box::<NoopXbxEngineInputBackend>::default(),
            Box::new(SharedStateXbxEngineWindowHost::new(window_state)),
        )));

        let stdin_handle = spawn_stdin_reader(
            sender.clone(),
            pending_host_responses.clone(),
            shutdown_flag.clone(),
        );
        let stdout_handle = spawn_stdout_writer(outgoing_messages.clone(), shutdown_flag.clone());
        let worker_handle = spawn_runtime_worker(
            app.clone(),
            receiver,
            outgoing_messages.clone(),
            shutdown_flag.clone(),
        );

        Ok((
            app,
            Self {
                sender,
                shutdown_flag,
                stdin_handle: Some(stdin_handle),
                stdout_handle: Some(stdout_handle),
                worker_handle: Some(worker_handle),
            },
        ))
    }

    pub fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        let _ = self.sender.send(WorkerCommand::Shutdown);
        // stdin 读取线程可能长期阻塞在 read_line，上层关闭窗口时不等待它退出，
        // 避免“原生窗口已关但进程还在等 stdin EOF”的假死。
        let _ = self.stdin_handle.take();
        if let Some(handle) = self.stdout_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

fn spawn_runtime_worker(
    app: Arc<Mutex<XbxEngineApp>>,
    receiver: mpsc::Receiver<WorkerCommand>,
    outgoing_messages: Arc<SharedOutgoingMessages>,
    shutdown_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        outgoing_messages.push(XbxEngineOutgoingMessageDto::Ready);

        loop {
            match receiver.recv_timeout(Duration::from_millis(16)) {
                Ok(WorkerCommand::ControlRequest {
                    request_id,
                    command,
                }) => {
                    let response = match app.lock().expect("lock stdio app").handle_control(command)
                    {
                        Ok(()) => XbxEngineOutgoingMessageDto::ControlResponse {
                            request_id,
                            response: XbxEngineControlResponseDto::Ack,
                        },
                        Err(error) => XbxEngineOutgoingMessageDto::ControlError {
                            request_id,
                            message: error.to_string(),
                        },
                    };
                    outgoing_messages.push(response);
                    drain_runtime_events(&app, &outgoing_messages);
                }
                Ok(WorkerCommand::Shutdown) => {
                    if let Ok(mut app) = app.lock() {
                        let _ = app.handle_control(XbxEngineControlCommandDto::StopRuntime);
                    }
                    drain_runtime_events(&app, &outgoing_messages);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Ok(mut app) = app.lock() {
                        app.tick();
                    }
                    drain_runtime_events(&app, &outgoing_messages);
                    if shutdown_flag.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    })
}

fn drain_runtime_events(
    app: &Arc<Mutex<XbxEngineApp>>,
    outgoing_messages: &Arc<SharedOutgoingMessages>,
) {
    if let Ok(mut app) = app.lock() {
        for event in app.drain_events() {
            outgoing_messages.push(XbxEngineOutgoingMessageDto::RuntimeEvent { event });
        }
    }
}

fn spawn_stdout_writer(
    outgoing_messages: Arc<SharedOutgoingMessages>,
    shutdown_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        loop {
            for message in outgoing_messages.drain() {
                if let Ok(line) = serde_json::to_string(&message) {
                    let _ = writeln!(stdout, "{line}");
                }
            }
            let _ = stdout.flush();

            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_millis(16));
        }
    })
}

fn spawn_stdin_reader(
    sender: mpsc::Sender<WorkerCommand>,
    pending_host_responses: Arc<Mutex<HashMap<String, mpsc::Sender<HostResponseResult>>>>,
    shutdown_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());

        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(message) = serde_json::from_str::<XbxEngineIncomingMessageDto>(line)
                    else {
                        continue;
                    };

                    match message {
                        XbxEngineIncomingMessageDto::ControlRequest {
                            request_id,
                            command,
                        } => {
                            let _ = sender.send(WorkerCommand::ControlRequest {
                                request_id,
                                command,
                            });
                        }
                        XbxEngineIncomingMessageDto::HostResponse {
                            request_id,
                            response,
                        } => {
                            if let Some(sender) = pending_host_responses
                                .lock()
                                .expect("lock pending host responses")
                                .remove(&request_id)
                            {
                                let _ = sender.send(Ok(response));
                            }
                        }
                        XbxEngineIncomingMessageDto::HostError {
                            request_id,
                            message,
                        } => {
                            if let Some(sender) = pending_host_responses
                                .lock()
                                .expect("lock pending host responses")
                                .remove(&request_id)
                            {
                                let _ = sender.send(Err(message));
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    })
}
