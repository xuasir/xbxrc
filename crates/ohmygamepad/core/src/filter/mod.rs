use crate::FilterConfig;

fn clamp_signed_unit(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

fn clamp_unsigned_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

pub(crate) fn filter_button(value: f32, config: &FilterConfig) -> f32 {
    let normalized = clamp_unsigned_unit(value);
    if normalized.abs() <= config.button_epsilon {
        0.0
    } else {
        normalized
    }
}

pub(crate) fn filter_trigger(value: f32, config: &FilterConfig) -> f32 {
    let normalized = clamp_unsigned_unit(value);
    if normalized <= config.trigger_deadzone {
        return 0.0;
    }

    let adjusted = (normalized - config.trigger_deadzone) / (1.0 - config.trigger_deadzone);
    if adjusted.abs() <= config.trigger_epsilon {
        0.0
    } else {
        adjusted.clamp(0.0, 1.0)
    }
}

pub(crate) fn filter_stick(x: f32, y: f32, config: &FilterConfig) -> (f32, f32) {
    let clamped_x = clamp_signed_unit(x);
    let clamped_y = clamp_signed_unit(y);
    let magnitude = (clamped_x * clamped_x + clamped_y * clamped_y).sqrt();
    if magnitude <= config.stick_deadzone {
        return (0.0, 0.0);
    }

    let scaled =
        ((magnitude - config.stick_deadzone) / (1.0 - config.stick_deadzone)).clamp(0.0, 1.0);
    let ratio = if magnitude > 0.0 {
        scaled / magnitude
    } else {
        0.0
    };
    let filtered_x = clamped_x * ratio;
    let filtered_y = clamped_y * ratio;

    (
        if filtered_x.abs() <= config.stick_epsilon {
            0.0
        } else {
            filtered_x.clamp(-1.0, 1.0)
        },
        if filtered_y.abs() <= config.stick_epsilon {
            0.0
        } else {
            filtered_y.clamp(-1.0, 1.0)
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::FilterConfig;

    use super::{filter_stick, filter_trigger};

    #[test]
    fn stick_values_inside_deadzone_are_zeroed() {
        let (x, y) = filter_stick(0.03, -0.05, &FilterConfig::default());
        assert_eq!((x, y), (0.0, 0.0));
    }

    #[test]
    fn stick_values_outside_deadzone_are_rescaled() {
        let (x, y) = filter_stick(0.5, 0.0, &FilterConfig::default());
        assert!(x > 0.4);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn trigger_values_inside_deadzone_are_zeroed() {
        assert_eq!(filter_trigger(0.01, &FilterConfig::default()), 0.0);
        assert_eq!(filter_trigger(0.03, &FilterConfig::default()), 0.0);
    }

    #[test]
    fn trigger_values_outside_deadzone_are_rescaled() {
        let trigger = filter_trigger(0.5, &FilterConfig::default());
        assert!(trigger > 0.4);
        assert!(trigger < 0.6);
    }
}
