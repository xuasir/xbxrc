use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

use ohmygamepad_gilrs::{GilrsInputEventKind, GilrsSource, RealGilrsSource};

fn main() -> Result<(), Box<dyn Error>> {
    let duration_secs = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(10);
    let duration_secs = duration_secs.max(1);

    let (mut source, _) = RealGilrsSource::new()?;
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(duration_secs);
    let mut per_second = PerSecondStats::default();
    let mut totals = TotalStats::default();
    let mut next_tick = started_at + Duration::from_secs(1);

    println!("=== OhMyGamepad Accuracy Probe ===");
    println!("duration: {duration_secs}s");
    println!("hint: move sticks + press ABXY / dpad / triggers now");
    println!();

    while Instant::now() < deadline {
        if let Some(event) = source.next_event() {
            per_second.events += 1;
            totals.events += 1;
            totals
                .devices
                .entry(event.device.device_id.clone())
                .or_insert_with(|| DeviceMeta {
                    name: event.device.name.clone(),
                    vendor_id: event.device.vendor_id,
                    product_id: event.device.product_id,
                });
            match event.kind {
                GilrsInputEventKind::Connected => {
                    per_second.connected += 1;
                    totals.connected += 1;
                }
                GilrsInputEventKind::Disconnected => {
                    per_second.disconnected += 1;
                    totals.disconnected += 1;
                }
                GilrsInputEventKind::ButtonChanged { index, value } => {
                    per_second.button_events += 1;
                    totals.button_events += 1;
                    if value > 0.5 {
                        per_second.buttons_pressed.insert(index);
                        totals.buttons_pressed.insert(index);
                    }
                }
                GilrsInputEventKind::AxisChanged { index, value } => {
                    per_second.axis_events += 1;
                    totals.axis_events += 1;
                    let peak = per_second.axis_peaks.entry(index).or_insert(0.0);
                    *peak = peak.max(value.abs());
                    let total_peak = totals.axis_peaks.entry(index).or_insert(0.0);
                    *total_peak = total_peak.max(value.abs());
                }
                GilrsInputEventKind::Dropped => {
                    per_second.dropped += 1;
                    totals.dropped += 1;
                }
            }
        } else {
            thread::sleep(Duration::from_millis(1));
        }

        let now = Instant::now();
        if now >= next_tick {
            print_tick((now - started_at).as_secs(), &per_second);
            per_second = PerSecondStats::default();
            next_tick += Duration::from_secs(1);
        }
    }

    println!();
    println!("=== Summary ===");
    println!(
        "events={} connected={} disconnected={} buttonEvents={} axisEvents={} dropped={}",
        totals.events,
        totals.connected,
        totals.disconnected,
        totals.button_events,
        totals.axis_events,
        totals.dropped
    );
    println!("activeButtons={:?}", totals.buttons_pressed);
    println!("axisPeaks={:?}", totals.axis_peaks);
    println!("devices:");
    for (device_id, meta) in &totals.devices {
        println!(
            "- id={} name=\"{}\" vid={} pid={}",
            device_id,
            meta.name,
            format_hex_opt(meta.vendor_id),
            format_hex_opt(meta.product_id)
        );
    }
    println!();
    println!("判定建议:");
    println!("- 如果总 events 约为 0，说明 gilrs 层没有采到输入（驱动/权限/设备层问题）。");
    println!("- 如果 events 很高但 activeButtons/axisPeaks 近乎空，说明映射索引可能错误。");
    println!("- 如果能稳定看到 buttonEvents + axisPeaks>0.6，采样准确性基本正常。");

    Ok(())
}

#[derive(Default)]
struct PerSecondStats {
    events: u64,
    connected: u64,
    disconnected: u64,
    button_events: u64,
    axis_events: u64,
    dropped: u64,
    buttons_pressed: BTreeSet<usize>,
    axis_peaks: BTreeMap<usize, f32>,
}

#[derive(Default)]
struct TotalStats {
    events: u64,
    connected: u64,
    disconnected: u64,
    button_events: u64,
    axis_events: u64,
    dropped: u64,
    buttons_pressed: BTreeSet<usize>,
    axis_peaks: BTreeMap<usize, f32>,
    devices: BTreeMap<String, DeviceMeta>,
}

struct DeviceMeta {
    name: String,
    vendor_id: Option<u16>,
    product_id: Option<u16>,
}

fn print_tick(second: u64, stats: &PerSecondStats) {
    println!(
        "[t+{:02}s] events={} btn={} axis={} conn={} disc={} dropped={} pressed={:?} axisPeaks={:?}",
        second,
        stats.events,
        stats.button_events,
        stats.axis_events,
        stats.connected,
        stats.disconnected,
        stats.dropped,
        stats.buttons_pressed,
        stats.axis_peaks
    );
}

fn format_hex_opt(value: Option<u16>) -> String {
    value
        .map(|v| format!("0x{v:04x}"))
        .unwrap_or_else(|| "null".to_owned())
}
