use ohmygamepad_protocol::{LogicalButtonsStateDto, LogicalPadStateDto, LogicalStickDto};

use crate::{
    filter_button, filter_stick, filter_trigger, AxisMapping, ButtonMapping, DeviceProfile,
    FilterConfig, RawDeviceSample,
};

fn read_button(sample: &RawDeviceSample, index: usize) -> f32 {
    sample.buttons.get(index).copied().unwrap_or(0.0)
}

fn read_axis(sample: &RawDeviceSample, index: usize) -> f32 {
    sample.axes.get(index).copied().unwrap_or(0.0)
}

fn read_trigger(
    sample: &RawDeviceSample,
    button_index: usize,
    axis_index: Option<usize>,
    filter: &FilterConfig,
) -> f32 {
    let button_value = read_button(sample, button_index);
    if button_value > 0.0 {
        return filter_trigger(button_value, filter);
    }

    let Some(axis_index) = axis_index else {
        return 0.0;
    };
    let axis_value = read_axis(sample, axis_index);
    if axis_value.abs() <= f32::EPSILON {
        return 0.0;
    }

    filter_trigger((axis_value + 1.0) * 0.5, filter)
}

pub(crate) fn default_logical_pad_state() -> LogicalPadStateDto {
    LogicalPadStateDto {
        buttons: LogicalButtonsStateDto::default(),
        left_stick: LogicalStickDto::default(),
        right_stick: LogicalStickDto::default(),
        left_trigger: 0.0,
        right_trigger: 0.0,
    }
}

fn default_profile() -> DeviceProfile {
    DeviceProfile::default()
}

pub(crate) fn map_standard_sample(sample: &RawDeviceSample) -> LogicalPadStateDto {
    map_sample_with_profile(sample, &default_profile())
}

pub(crate) fn map_sample_with_profile(
    sample: &RawDeviceSample,
    profile: &DeviceProfile,
) -> LogicalPadStateDto {
    let ButtonMapping {
        south,
        east,
        west,
        north,
        l1,
        r1,
        l2,
        r2,
        view,
        menu,
        l3,
        r3,
        dpad_up,
        dpad_down,
        dpad_left,
        dpad_right,
        home,
    } = profile.buttons;
    let AxisMapping {
        left_stick_x,
        left_stick_y,
        right_stick_x,
        right_stick_y,
        left_trigger_button,
        right_trigger_button,
        left_trigger_axis,
        right_trigger_axis,
    } = profile.axes;
    let filter = &profile.filter;

    let (left_x, left_y) = filter_stick(
        read_axis(sample, left_stick_x),
        read_axis(sample, left_stick_y),
        filter,
    );
    let (right_x, right_y) = filter_stick(
        read_axis(sample, right_stick_x),
        read_axis(sample, right_stick_y),
        filter,
    );

    LogicalPadStateDto {
        buttons: LogicalButtonsStateDto {
            south: filter_button(read_button(sample, south), filter),
            east: filter_button(read_button(sample, east), filter),
            west: filter_button(read_button(sample, west), filter),
            north: filter_button(read_button(sample, north), filter),
            l1: filter_button(read_button(sample, l1), filter),
            r1: filter_button(read_button(sample, r1), filter),
            l2: filter_button(read_button(sample, l2), filter),
            r2: filter_button(read_button(sample, r2), filter),
            view: filter_button(read_button(sample, view), filter),
            menu: filter_button(read_button(sample, menu), filter),
            l3: filter_button(read_button(sample, l3), filter),
            r3: filter_button(read_button(sample, r3), filter),
            dpad_up: filter_button(read_button(sample, dpad_up), filter),
            dpad_down: filter_button(read_button(sample, dpad_down), filter),
            dpad_left: filter_button(read_button(sample, dpad_left), filter),
            dpad_right: filter_button(read_button(sample, dpad_right), filter),
            home: filter_button(read_button(sample, home), filter),
        },
        left_stick: LogicalStickDto {
            x: left_x,
            y: left_y,
        },
        right_stick: LogicalStickDto {
            x: right_x,
            y: right_y,
        },
        left_trigger: read_trigger(sample, left_trigger_button, left_trigger_axis, filter),
        right_trigger: read_trigger(sample, right_trigger_button, right_trigger_axis, filter),
    }
}

pub(crate) fn merge_states(states: &[LogicalPadStateDto]) -> LogicalPadStateDto {
    let mut merged = default_logical_pad_state();
    for state in states {
        merged.buttons.south = merged.buttons.south.max(state.buttons.south);
        merged.buttons.east = merged.buttons.east.max(state.buttons.east);
        merged.buttons.west = merged.buttons.west.max(state.buttons.west);
        merged.buttons.north = merged.buttons.north.max(state.buttons.north);
        merged.buttons.l1 = merged.buttons.l1.max(state.buttons.l1);
        merged.buttons.r1 = merged.buttons.r1.max(state.buttons.r1);
        merged.buttons.l2 = merged.buttons.l2.max(state.buttons.l2);
        merged.buttons.r2 = merged.buttons.r2.max(state.buttons.r2);
        merged.buttons.l3 = merged.buttons.l3.max(state.buttons.l3);
        merged.buttons.r3 = merged.buttons.r3.max(state.buttons.r3);
        merged.buttons.view = merged.buttons.view.max(state.buttons.view);
        merged.buttons.menu = merged.buttons.menu.max(state.buttons.menu);
        merged.buttons.home = merged.buttons.home.max(state.buttons.home);
        merged.buttons.dpad_up = merged.buttons.dpad_up.max(state.buttons.dpad_up);
        merged.buttons.dpad_down = merged.buttons.dpad_down.max(state.buttons.dpad_down);
        merged.buttons.dpad_left = merged.buttons.dpad_left.max(state.buttons.dpad_left);
        merged.buttons.dpad_right = merged.buttons.dpad_right.max(state.buttons.dpad_right);
        merged.left_stick.x = pick_stronger_axis(merged.left_stick.x, state.left_stick.x);
        merged.left_stick.y = pick_stronger_axis(merged.left_stick.y, state.left_stick.y);
        merged.right_stick.x = pick_stronger_axis(merged.right_stick.x, state.right_stick.x);
        merged.right_stick.y = pick_stronger_axis(merged.right_stick.y, state.right_stick.y);
        merged.left_trigger = merged.left_trigger.max(state.left_trigger);
        merged.right_trigger = merged.right_trigger.max(state.right_trigger);
    }
    merged
}

fn pick_stronger_axis(current: f32, incoming: f32) -> f32 {
    if incoming.abs() > current.abs() {
        incoming
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::{
        default_logical_pad_state, map_sample_with_profile, map_standard_sample, merge_states,
    };
    use crate::{ButtonMapping, DeviceProfile, FilterConfig, RawDeviceSample};

    fn sample(buttons: Vec<f32>, axes: Vec<f32>) -> RawDeviceSample {
        RawDeviceSample {
            device_id: "pad-a".to_owned(),
            observed_at_ms: 1,
            buttons,
            axes,
        }
    }

    #[test]
    fn standard_mapping_uses_default_button_layout() {
        let mapped = map_standard_sample(&sample(
            vec![
                1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.8, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0,
            ],
            vec![0.5, -0.5, 0.0, 0.0],
        ));

        assert_eq!(mapped.buttons.south, 1.0);
        assert_eq!(mapped.buttons.west, 0.5);
        assert_eq!(mapped.buttons.l1, 1.0);
        assert!(mapped.left_trigger > 0.7);
        assert_eq!(mapped.buttons.menu, 1.0);
        assert_eq!(mapped.buttons.dpad_up, 1.0);
        assert_eq!(mapped.buttons.dpad_right, 1.0);
        assert_eq!(mapped.buttons.home, 1.0);
        assert!(mapped.left_stick.x > 0.4);
        assert!(mapped.left_stick.y < -0.4);
    }

    #[test]
    fn merge_prefers_stronger_axes_and_max_button_pressure() {
        let merged = merge_states(&[
            map_standard_sample(&sample(vec![0.4], vec![0.2, 0.0, 0.0, 0.0])),
            map_standard_sample(&sample(vec![1.0, 0.7], vec![-0.7, 0.0, 0.0, 0.0])),
        ]);

        assert_eq!(merged.buttons.south, 1.0);
        assert_eq!(merged.buttons.east, 0.7);
        assert!(merged.left_stick.x < -0.6);
    }

    #[test]
    fn default_state_is_zeroed() {
        let state = default_logical_pad_state();
        assert_eq!(state.buttons.south, 0.0);
        assert_eq!(state.right_trigger, 0.0);
    }

    #[test]
    fn custom_profile_can_remap_primary_button() {
        let mapped = map_sample_with_profile(
            &sample(vec![0.0, 0.0, 0.0, 1.0], vec![0.0, 0.0, 0.0, 0.0]),
            &DeviceProfile {
                buttons: ButtonMapping {
                    south: 3,
                    north: 0,
                    ..ButtonMapping::default()
                },
                ..DeviceProfile::default()
            },
        );

        assert_eq!(mapped.buttons.south, 1.0);
        assert_eq!(mapped.buttons.north, 0.0);
    }

    #[test]
    fn custom_profile_can_override_deadzone() {
        let mapped = map_sample_with_profile(
            &sample(vec![0.0], vec![0.05, 0.0, 0.0, 0.0]),
            &DeviceProfile {
                filter: FilterConfig {
                    stick_deadzone: 0.0,
                    stick_epsilon: 0.0,
                    trigger_deadzone: 0.03,
                    trigger_epsilon: 0.01,
                    button_epsilon: 0.0001,
                },
                ..DeviceProfile::default()
            },
        );

        assert!(mapped.left_stick.x > 0.04);
    }
}
