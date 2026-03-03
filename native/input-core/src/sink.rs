use input_dto::{GamepadDeviceDto, LogicalPadSnapshotDto};

pub trait UiSink {
    fn emit_devices_changed(&mut self, devices: &[GamepadDeviceDto]);
    fn emit_pad_snapshot(&mut self, snapshot: &LogicalPadSnapshotDto);
}

pub trait StreamSink {
    fn emit_pad_snapshot(&mut self, snapshot: &LogicalPadSnapshotDto);
}

#[derive(Default)]
pub struct NoopUiSink;

impl UiSink for NoopUiSink {
    fn emit_devices_changed(&mut self, _devices: &[GamepadDeviceDto]) {}

    fn emit_pad_snapshot(&mut self, _snapshot: &LogicalPadSnapshotDto) {}
}

#[derive(Default)]
pub struct NoopStreamSink;

impl StreamSink for NoopStreamSink {
    fn emit_pad_snapshot(&mut self, _snapshot: &LogicalPadSnapshotDto) {}
}
