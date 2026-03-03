use input_dto::{
    GamepadBridgeCommandDto, GamepadBridgeEventDto, GamepadDeviceDto, GamepadRouteTargetDto,
    GamepadRuntimeSnapshotDto, GamepadSamplingConfigDto, LogicalPadBindingDto,
    LogicalPadSnapshotDto,
};

pub trait BridgePublisher {
    fn publish(&mut self, event: GamepadBridgeEventDto);
}

#[derive(Clone, Debug, PartialEq)]
pub enum BridgeAction {
    RefreshRuntimeSnapshot,
    SetRouteTarget(GamepadRouteTargetDto),
    UpdateSampling(GamepadSamplingConfigDto),
    RebindLogicalPad(LogicalPadBindingDto),
}

#[derive(Default)]
pub struct NoopBridgePublisher;

impl BridgePublisher for NoopBridgePublisher {
    fn publish(&mut self, _event: GamepadBridgeEventDto) {}
}

pub struct InputBridge<TPublisher> {
    publisher: TPublisher,
}

impl<TPublisher> InputBridge<TPublisher>
where
    TPublisher: BridgePublisher,
{
    pub fn new(publisher: TPublisher) -> Self {
        Self { publisher }
    }

    pub fn translate_command(command: GamepadBridgeCommandDto) -> BridgeAction {
        match command {
            GamepadBridgeCommandDto::RefreshRuntimeSnapshot => BridgeAction::RefreshRuntimeSnapshot,
            GamepadBridgeCommandDto::SetRouteTarget { target } => {
                BridgeAction::SetRouteTarget(target)
            }
            GamepadBridgeCommandDto::UpdateSampling { sampling } => {
                BridgeAction::UpdateSampling(sampling)
            }
            GamepadBridgeCommandDto::RebindLogicalPad { binding } => {
                BridgeAction::RebindLogicalPad(binding)
            }
        }
    }

    pub fn publish_runtime_snapshot(&mut self, snapshot: GamepadRuntimeSnapshotDto) {
        self.publisher
            .publish(GamepadBridgeEventDto::RuntimeSnapshot { snapshot });
    }

    pub fn publish_devices_changed(&mut self, devices: Vec<GamepadDeviceDto>) {
        self.publisher
            .publish(GamepadBridgeEventDto::DevicesChanged { devices });
    }

    pub fn publish_pad_snapshot(&mut self, snapshot: LogicalPadSnapshotDto) {
        self.publisher
            .publish(GamepadBridgeEventDto::PadSnapshot { snapshot });
    }

    pub fn publish_route_changed(&mut self, target: GamepadRouteTargetDto) {
        self.publisher
            .publish(GamepadBridgeEventDto::RouteChanged { target });
    }

    pub fn into_inner(self) -> TPublisher {
        self.publisher
    }
}
