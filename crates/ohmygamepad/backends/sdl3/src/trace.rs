use std::sync::{Arc, Mutex, OnceLock};

type RuntimeTraceSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

fn runtime_trace_sink_slot() -> &'static Mutex<Option<RuntimeTraceSink>> {
    static SLOT: OnceLock<Mutex<Option<RuntimeTraceSink>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

pub fn set_runtime_trace_sink(sink: Option<RuntimeTraceSink>) {
    if let Ok(mut slot) = runtime_trace_sink_slot().lock() {
        *slot = sink;
    }
}

pub fn record_runtime_trace(event: &str, payload: serde_json::Value) {
    let sink = runtime_trace_sink_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if let Some(sink) = sink {
        sink(event, payload);
    }
}
