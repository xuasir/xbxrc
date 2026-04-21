#[cfg(windows)]
fn main() {
    use std::thread;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::Input::XboxController::{
        XInputGetState, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK,
        XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT,
        XINPUT_GAMEPAD_DPAD_UP, XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB,
        XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START,
        XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y, XINPUT_STATE,
    };

    let duration_secs = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(10)
        .max(1);
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(duration_secs);

    println!("=== XInput Accuracy Probe ===");
    println!("duration: {duration_secs}s");
    println!("hint: press ABXY / dpad / triggers / sticks now");
    println!();

    let mut last_packet = 0u32;
    let mut ticks = 0u64;
    let mut changed_ticks = 0u64;
    let mut connected_user: Option<u32> = None;

    while Instant::now() < deadline {
        let mut found = false;
        for user_idx in 0..=3u32 {
            let mut state = XINPUT_STATE {
                dwPacketNumber: 0,
                Gamepad: Default::default(),
            };
            // SAFETY: XInputGetState is an FFI call with valid pointers/indices.
            let result = unsafe { XInputGetState(user_idx, &mut state as *mut XINPUT_STATE) };
            if result == 0 {
                found = true;
                if connected_user != Some(user_idx) {
                    connected_user = Some(user_idx);
                    println!("[connect] user={} packet={}", user_idx, state.dwPacketNumber);
                    last_packet = state.dwPacketNumber;
                }

                ticks += 1;
                let changed = state.dwPacketNumber != last_packet;
                if changed {
                    changed_ticks += 1;
                    last_packet = state.dwPacketNumber;
                }

                let gp = state.Gamepad;
                println!(
                    "[t+{:02}s] user={} packet={} changed={} buttons={} LT={} RT={} LS=({}, {}) RS=({}, {})",
                    (Instant::now() - started_at).as_secs(),
                    user_idx,
                    state.dwPacketNumber,
                    changed,
                    format_buttons(gp.wButtons),
                    gp.bLeftTrigger,
                    gp.bRightTrigger,
                    gp.sThumbLX,
                    gp.sThumbLY,
                    gp.sThumbRX,
                    gp.sThumbRY
                );
                break;
            }
        }

        if !found {
            println!(
                "[t+{:02}s] disconnected (no xinput user 0..3)",
                (Instant::now() - started_at).as_secs()
            );
        }
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
    println!("- 如果这里 changedTicks > 0，而 gilrs probe 仍是 0，说明是 gilrs 通道问题。");
    println!("- 如果这里也始终 0，说明系统输入链路/驱动层异常。");

    const _: u16 = XINPUT_GAMEPAD_A
        | XINPUT_GAMEPAD_B
        | XINPUT_GAMEPAD_X
        | XINPUT_GAMEPAD_Y
        | XINPUT_GAMEPAD_DPAD_UP
        | XINPUT_GAMEPAD_DPAD_DOWN
        | XINPUT_GAMEPAD_DPAD_LEFT
        | XINPUT_GAMEPAD_DPAD_RIGHT
        | XINPUT_GAMEPAD_LEFT_SHOULDER
        | XINPUT_GAMEPAD_RIGHT_SHOULDER
        | XINPUT_GAMEPAD_START
        | XINPUT_GAMEPAD_BACK
        | XINPUT_GAMEPAD_LEFT_THUMB
        | XINPUT_GAMEPAD_RIGHT_THUMB;
}

#[cfg(windows)]
fn format_buttons(mask: u16) -> String {
    let mut names = Vec::new();
    push_button(&mut names, mask, "A", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_A);
    push_button(&mut names, mask, "B", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_B);
    push_button(&mut names, mask, "X", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_X);
    push_button(&mut names, mask, "Y", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_Y);
    push_button(&mut names, mask, "DUp", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_DPAD_UP);
    push_button(&mut names, mask, "DDown", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_DPAD_DOWN);
    push_button(&mut names, mask, "DLeft", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_DPAD_LEFT);
    push_button(&mut names, mask, "DRight", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_DPAD_RIGHT);
    push_button(&mut names, mask, "LB", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_LEFT_SHOULDER);
    push_button(&mut names, mask, "RB", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_RIGHT_SHOULDER);
    push_button(&mut names, mask, "Back", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_BACK);
    push_button(&mut names, mask, "Start", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_START);
    push_button(&mut names, mask, "LS", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_LEFT_THUMB);
    push_button(&mut names, mask, "RS", windows_sys::Win32::UI::Input::XboxController::XINPUT_GAMEPAD_RIGHT_THUMB);
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(",")
    }
}

#[cfg(windows)]
fn push_button(buf: &mut Vec<&'static str>, mask: u16, label: &'static str, flag: u16) {
    if mask & flag != 0 {
        buf.push(label);
    }
}

#[cfg(not(windows))]
fn main() {
    println!("xinput_accuracy_probe is only available on Windows.");
}
