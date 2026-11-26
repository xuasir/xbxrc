import Foundation
@preconcurrency import Metal

struct StreamingRenderResourceCounters: Equatable, Sendable {
    var pipelineLoadCount: Int64 = 0
    var pipelineCreateCount: Int64 = 0
    var textureImportCount: Int64 = 0
    var sameFrameTextureReuseCount: Int64 = 0
    var intermediateTextureCreateCount: Int64 = 0
    var uniformBuildCount: Int64 = 0
    var renderPassDescriptorCreateCount: Int64 = 2
    var cacheResetCount: Int64 = 0
    var cacheTrimCount: Int64 = 0
    var applicationHeldFrameCount: Int64 = 0
    var activeGpuTextureBytes: Int64 = 0

    func tracePayload() -> [String: IOSRuntimeTraceValue] {
        [
            "pipelineLoadCount": .integer(pipelineLoadCount),
            "pipelineCreateCount": .integer(pipelineCreateCount),
            "textureImportCount": .integer(textureImportCount),
            "sameFrameTextureReuseCount": .integer(sameFrameTextureReuseCount),
            "intermediateTextureCreateCount": .integer(intermediateTextureCreateCount),
            "uniformBuildCount": .integer(uniformBuildCount),
            "renderPassDescriptorCreateCount": .integer(renderPassDescriptorCreateCount),
            "cacheResetCount": .integer(cacheResetCount),
            "cacheTrimCount": .integer(cacheTrimCount),
            "applicationHeldFrameCount": .integer(applicationHeldFrameCount),
            "activeGpuTextureBytes": .integer(activeGpuTextureBytes),
        ]
    }
}

/// 不可变 Metal 能力句柄；编码、预热和缓存变更都归 cache.queue。
struct StreamingRenderPipelines: @unchecked Sendable {
    let device: MTLDevice
    let commandQueue: MTLCommandQueue
    let nv12: MTLRenderPipelineState
    let i420: MTLRenderPipelineState
    let bgra: MTLRenderPipelineState
}

final class StreamingRenderResourceCache: Sendable {
    let queue = DispatchQueue(label: "xbxrc.ios.streaming.render", qos: .userInteractive)
    private nonisolated(unsafe) var entries: [(key: UInt64, resources: StreamingRenderPipelines)] =
        []
    private nonisolated(unsafe) var loadCount: Int64 = 0
    private nonisolated(unsafe) var createCount: Int64 = 0

    func resources(on device: MTLDevice) -> StreamingRenderPipelines? {
        dispatchPrecondition(condition: .onQueue(queue))
        if let index = entries.firstIndex(where: { $0.key == device.registryID }) {
            let entry = entries.remove(at: index)
            entries.append(entry)
            return entry.resources
        }
        guard let commandQueue = device.makeCommandQueue(),
            let library = device.makeDefaultLibrary(),
            let vertex = library.makeFunction(name: "streamingVertex")
        else { return nil }
        loadCount += 1
        func pipeline(_ name: String) -> MTLRenderPipelineState? {
            let descriptor = MTLRenderPipelineDescriptor()
            descriptor.vertexFunction = vertex
            descriptor.fragmentFunction = library.makeFunction(name: name)
            descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
            guard descriptor.fragmentFunction != nil else { return nil }
            return try? device.makeRenderPipelineState(descriptor: descriptor)
        }
        guard let nv12 = pipeline("streamingNV12Fragment"),
            let i420 = pipeline("streamingI420Fragment"),
            let bgra = pipeline("streamingBGRAFragment")
        else { return nil }
        createCount += 3
        let result = StreamingRenderPipelines(
            device: device, commandQueue: commandQueue, nv12: nv12, i420: i420, bgra: bgra)
        if entries.count == 4 { entries.removeFirst() }
        entries.append((device.registryID, result))
        return result
    }

    var counts: (loaded: Int64, created: Int64) {
        dispatchPrecondition(condition: .onQueue(queue))
        return (loadCount, createCount)
    }

    func trim() {
        dispatchPrecondition(condition: .onQueue(queue))
        entries.removeAll(keepingCapacity: true)
    }

    static func supportsPictureRendering(on device: MTLDevice) -> Bool {
        // 预编译资源存在即可进入播放能力检查；pipeline 由 RenderQueue 单次加载。
        device.supportsFamily(.apple1)
            && Bundle.main.url(forResource: "default", withExtension: "metallib") != nil
    }
}
