import CoreVideo
@preconcurrency import Metal

/// 强引用对应帧的 buffer 与 CVMetalTexture views，完成票据持有到 GPU completion。
struct ImportedFrameResources: @unchecked Sendable {
    let frameID: UInt64
    let renderEpoch: UInt64
    let storage: StreamingDecodedFrameStorage
    let textureRefs: [CVMetalTexture]
    let primary: MTLTexture
    let secondary: MTLTexture?
    let tertiary: MTLTexture?
    let pipeline: MTLRenderPipelineState

    var textureBytes: Int64 {
        Int64(
            primary.allocatedSize + (secondary?.allocatedSize ?? 0) + (tertiary?.allocatedSize ?? 0)
        )
    }
}
