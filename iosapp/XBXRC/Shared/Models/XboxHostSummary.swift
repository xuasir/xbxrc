import Foundation

struct XboxHostSummary: Identifiable, Equatable, Hashable, Sendable {
    let id: String
    let commandID: String?
    let streamTargetID: String?
    let name: String
    let consoleType: String
    let locale: String?
    let region: String?
    let powerState: String?
    let remoteManagementEnabled: Bool?
    let consoleStreamingEnabled: Bool?
    let wirelessWarning: Bool?
    let outOfHomeWarning: Bool?
    let storageDevices: [XboxHostStorageSummary]

    var statusTitle: String {
        switch powerState {
        case "On": "已开机"
        case "ConnectedStandby", "Connected": "待机可用"
        case "Off": "已关机"
        default: "状态未知"
        }
    }

    var readinessDescription: String {
        if powerState == "On", consoleStreamingEnabled != false {
            return "主机已准备好进行远程游玩"
        }
        if powerState == "ConnectedStandby" || powerState == "Connected",
           remoteManagementEnabled == true {
            return "可以通过远程管理唤醒主机"
        }
        if powerState == "Off" {
            return "请检查主机电源与网络连接"
        }
        return "远程游玩状态仍在同步"
    }

    var canStartRemotePlay: Bool {
        guard streamTargetID != nil else { return false }
        if powerState == "On" {
            return consoleStreamingEnabled != false
        }
        if powerState == "Connected" || powerState == "ConnectedStandby" {
            return remoteManagementEnabled == true
        }
        return false
    }
}

struct XboxHostStorageSummary: Identifiable, Equatable, Hashable, Sendable {
    let id: String
    let name: String
    let freeBytes: UInt64?
    let totalBytes: UInt64?

    var usedFraction: Double? {
        guard let freeBytes, let totalBytes, totalBytes > 0 else { return nil }
        return min(1, max(0, Double(totalBytes - min(freeBytes, totalBytes)) / Double(totalBytes)))
    }
}
