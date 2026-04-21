#[cfg(target_os = "windows")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::thread;
    use std::time::{Duration, Instant};
    use windows::Gaming::Input::Gamepad;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    let duration_secs = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(10)
        .max(1);

    // WinRT 输入调用前确保线程进入 MTA。
    let _ = unsafe { CoInitializeEx(Some(std::ptr::null()), COINIT_MULTITHREADED) };

    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(duration_secs);
    let mut ticks = 0u64;
    let mut changed_ticks = 0u64;
    let mut last_packet_sig: Option<u64> = None;
    let mut non_zero_timestamp_seen = false;

    println!("=== WinRT Gamepad Probe ===");
    println!("duration: {duration_secs}s");
    println!("hint: press ABXY / dpad / triggers / sticks now");
    println!();

    while Instant::now() < deadline {
        let gamepads = Gamepad::Gamepads()?;
        let count = gamepads.Size()?;
        if count == 0 {
            println!(
                "[t+{:02}s] gamepads=0",
                (Instant::now() - started_at).as_secs()
            );
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let gamepad = gamepads.GetAt(0)?;
        let reading = gamepad.GetCurrentReading()?;
        let sig = reading.Timestamp;
        if sig != 0 {
            non_zero_timestamp_seen = true;
        }
        let changed = last_packet_sig != Some(sig);
        if changed {
            changed_ticks += 1;
            last_packet_sig = Some(sig);
        }
        ticks += 1;

        println!(
            "[t+{:02}s] gamepads={} changed={} ts={} buttons={:#x} LT={:.3} RT={:.3} LS=({:.3},{:.3}) RS=({:.3},{:.3})",
            (Instant::now() - started_at).as_secs(),
            count,
            changed,
            sig,
            reading.Buttons.0,
            reading.LeftTrigger,
            reading.RightTrigger,
            reading.LeftThumbstickX,
            reading.LeftThumbstickY,
            reading.RightThumbstickX,
            reading.RightThumbstickY
        );

        thread::sleep(Duration::from_millis(100));
    }

    println!();
    println!(
        "summary: ticks={} changedTicks={} changeRate={:.2}%",
        ticks,
        changed_ticks,
        if ticks == 0 {
            0.0
        } else {
            (changed_ticks as f64 / ticks as f64) * 100.0
        }
    );
    let effective_changed_ticks = changed_ticks.saturating_sub(1);
    let effective_rate = if ticks <= 1 {
        0.0
    } else {
        (effective_changed_ticks as f64 / (ticks - 1) as f64) * 100.0
    };
    println!(
        "effective: changedTicks(excluding first)={} changeRate={:.2}% timestampNonZeroSeen={}",
        effective_changed_ticks, effective_rate, non_zero_timestamp_seen
    );
    println!("- 判定可用建议：timestampNonZeroSeen=true 且 effective changeRate >= 5%.");
    println!("- 否则可视为 WinRT 输入不可用（即使枚举到 gamepads）。");

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() {
    println!("winrt_gamepad_probe is only available on Windows.");
}
