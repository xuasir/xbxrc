use std::{error::Error, thread, time::Duration};

use ohmygamepad_gilrs::{
    NoopGilrsSource, OhMyGamepadDesktopKeyboardListener, OhMyGamepadKeyboardKey,
    OhMyGamepadService, OhMyGamepadServiceConfig,
};
use ohmygamepad_protocol::{LogicalPadStateDto, LogicalStickDto};

fn main() -> Result<(), Box<dyn Error>> {
    let service = OhMyGamepadService::spawn_with_source(
        OhMyGamepadServiceConfig {
            desktop_keyboard: None,
            ..OhMyGamepadServiceConfig::default()
        },
        NoopGilrsSource,
    );
    let mut listener = OhMyGamepadDesktopKeyboardListener::default();

    print_help(listener.poll_interval());

    loop {
        let state = listener
            .submit_to_service(&service)
            .map_err(|error| format!("failed to submit keyboard state: {error:?}"))?;
        let snapshot = service
            .snapshot()
            .map_err(|error| format!("failed to fetch runtime snapshot: {error:?}"))?;

        print!("\x1B[2J\x1B[H");
        print_help(listener.poll_interval());
        print_runtime(&snapshot, &state, listener.pressed_keys());

        if listener.is_pressed(OhMyGamepadKeyboardKey::KeyQ)
            || listener.is_pressed(OhMyGamepadKeyboardKey::Escape)
        {
            break;
        }

        thread::sleep(listener.poll_interval());
    }

    service
        .shutdown()
        .map_err(|error| format!("failed to shutdown service: {error:?}"))?;
    Ok(())
}

fn print_help(poll_interval: Duration) {
    println!("OhMyGamepad keyboard debug");
    println!("This example uses device_query to poll desktop keyboard state.");
    println!(
        "Poll interval: {} ms. Hold Q or Esc to quit.",
        poll_interval.as_millis()
    );
    println!();
    println!("Left stick : W/A/S/D");
    println!("Right stick: T/F/G/H");
    println!("Face       : J=South  K=East  U=West  I=North");
    println!("Shoulders  : 1=L1  2=R1  3=LT  4=RT");
    println!("Meta       : Z=L3  X=R3  Tab/7=View  Enter/8=Menu  9=Home");
    println!("DPad       : Arrow keys");
    println!("Exit       : Q or Esc");
    println!();
}

fn print_runtime(
    snapshot: &ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto,
    state: &LogicalPadStateDto,
    pressed_keys: &std::collections::BTreeSet<OhMyGamepadKeyboardKey>,
) {
    let pad = snapshot.pads.first();

    println!(
        "Active devices: {:?}",
        snapshot
            .devices
            .iter()
            .map(|device| &device.device_id)
            .collect::<Vec<_>>()
    );
    println!(
        "Pressed keys : {:?}",
        pressed_keys.iter().collect::<Vec<_>>()
    );
    println!(
        "Pad0 devices : {:?}",
        pad.map(|item| item.device_ids.clone()).unwrap_or_default()
    );
    println!();
    println!("Buttons: {}", format_buttons(state));
    println!("LeftStick : {}", format_stick("LS", state.left_stick));
    println!("RightStick: {}", format_stick("RS", state.right_stick));
    println!(
        "Triggers  : LT={:.1} RT={:.1}",
        state.left_trigger, state.right_trigger
    );

    if let Some(pad) = pad {
        println!();
        println!(
            "Runtime mirror: buttons={}, LS={}, RS={}, LT={:.1}, RT={:.1}",
            format_buttons(&pad.state),
            format_stick("LS", pad.state.left_stick),
            format_stick("RS", pad.state.right_stick),
            pad.state.left_trigger,
            pad.state.right_trigger
        );
    }
}

fn format_buttons(state: &LogicalPadStateDto) -> String {
    let mut names = Vec::new();

    push_if_active(&mut names, "South(J)", state.buttons.south);
    push_if_active(&mut names, "East(K)", state.buttons.east);
    push_if_active(&mut names, "West(U)", state.buttons.west);
    push_if_active(&mut names, "North(I)", state.buttons.north);
    push_if_active(&mut names, "L1(1)", state.buttons.l1);
    push_if_active(&mut names, "R1(2)", state.buttons.r1);
    push_if_active(&mut names, "View(Tab/7)", state.buttons.view);
    push_if_active(&mut names, "Menu(Enter/8)", state.buttons.menu);
    push_if_active(&mut names, "Home(9)", state.buttons.home);
    push_if_active(&mut names, "L3(Z)", state.buttons.l3);
    push_if_active(&mut names, "R3(X)", state.buttons.r3);
    push_if_active(&mut names, "DPadUp", state.buttons.dpad_up);
    push_if_active(&mut names, "DPadDown", state.buttons.dpad_down);
    push_if_active(&mut names, "DPadLeft", state.buttons.dpad_left);
    push_if_active(&mut names, "DPadRight", state.buttons.dpad_right);

    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

fn push_if_active(items: &mut Vec<&'static str>, label: &'static str, value: f32) {
    if value > 0.5 {
        items.push(label);
    }
}

fn format_stick(label: &str, stick: LogicalStickDto) -> String {
    format!("{label}(x={:.1}, y={:.1})", stick.x, stick.y)
}
