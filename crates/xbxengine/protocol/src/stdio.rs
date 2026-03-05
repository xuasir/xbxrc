use serde::{Deserialize, Serialize};

use crate::{
    XbxEngineControlCommandDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineRuntimeEventDto, XbxEngineStatsDto,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XbxEngineControlResponseDto {
    Ack,
    Stats { stats: XbxEngineStatsDto },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum XbxEngineIncomingMessageDto {
    ControlRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        command: XbxEngineControlCommandDto,
    },
    HostResponse {
        #[serde(rename = "requestId")]
        request_id: String,
        response: XbxEngineHostResponseDto,
    },
    HostError {
        #[serde(rename = "requestId")]
        request_id: String,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum XbxEngineOutgoingMessageDto {
    Ready,
    ControlResponse {
        #[serde(rename = "requestId")]
        request_id: String,
        response: XbxEngineControlResponseDto,
    },
    ControlError {
        #[serde(rename = "requestId")]
        request_id: String,
        message: String,
    },
    RuntimeEvent {
        event: XbxEngineRuntimeEventDto,
    },
    HostRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        request: XbxEngineHostRequestDto,
    },
}
