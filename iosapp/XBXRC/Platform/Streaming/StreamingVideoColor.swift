import CoreGraphics
import CoreVideo
import Foundation
import simd

enum StreamingYCbCrMatrix: String, Equatable, Sendable {
    case bt601, bt709, bt2020

    var coefficients: (red: Float, blue: Float) {
        switch self {
        case .bt601: (0.299, 0.114)
        case .bt709: (0.2126, 0.0722)
        case .bt2020: (0.2627, 0.0593)
        }
    }
}

struct StreamingVideoColorConfiguration: Equatable, Sendable {
    let matrix: StreamingYCbCrMatrix
    let fullRange: Bool
    let primaries: String
    let transferFunction: String
    let matrixSource: String

    init(pixelBuffer: CVPixelBuffer, width: Int, height: Int, fullRange: Bool) {
        func attachment(_ key: CFString) -> String? {
            CVBufferCopyAttachment(pixelBuffer, key, nil) as? String
        }
        self.init(
            width: width, height: height, fullRange: fullRange,
            matrixAttachment: attachment(kCVImageBufferYCbCrMatrixKey),
            primariesAttachment: attachment(kCVImageBufferColorPrimariesKey),
            transferAttachment: attachment(kCVImageBufferTransferFunctionKey),
            rgb: CVPixelBufferGetPixelFormatType(pixelBuffer) == kCVPixelFormatType_32BGRA
        )
    }

    init(
        width: Int, height: Int, fullRange: Bool,
        matrixAttachment: String? = nil, primariesAttachment: String? = nil,
        transferAttachment: String? = nil, rgb: Bool = false
    ) {
        let explicitMatrix: StreamingYCbCrMatrix?
        if matrixAttachment == (kCVImageBufferYCbCrMatrix_ITU_R_601_4 as String) {
            explicitMatrix = .bt601
        } else if matrixAttachment == (kCVImageBufferYCbCrMatrix_ITU_R_709_2 as String) {
            explicitMatrix = .bt709
        } else if matrixAttachment == (kCVImageBufferYCbCrMatrix_ITU_R_2020 as String) {
            explicitMatrix = .bt2020
        } else {
            explicitMatrix = nil
        }
        matrix = explicitMatrix ?? (width >= 1280 || height >= 720 ? .bt709 : .bt601)
        matrixSource = explicitMatrix == nil ? "resolutionFallback" : "pixelBufferAttachment"
        self.fullRange = fullRange
        primaries =
            primariesAttachment
            ?? (matrix == .bt2020
                ? kCVImageBufferColorPrimaries_ITU_R_2020 as String
                : matrix == .bt709 || rgb
                    ? kCVImageBufferColorPrimaries_ITU_R_709_2 as String
                    : kCVImageBufferColorPrimaries_SMPTE_C as String)
        transferFunction =
            transferAttachment
            ?? (rgb
                ? kCVImageBufferTransferFunction_sRGB as String
                : kCVImageBufferTransferFunction_ITU_R_709_2 as String)
    }

    var conversion: (matrix: simd_float3x3, offset: SIMD3<Float>) {
        let (red, blue) = matrix.coefficients
        let green = 1 - red - blue
        let lumaScale: Float = fullRange ? 1 : 255.0 / 219.0
        let chromaScale: Float = fullRange ? 1 : 255.0 / 224.0
        return (
            simd_float3x3(rows: [
                SIMD3<Float>(lumaScale, 0, 2 * (1 - red) * chromaScale),
                SIMD3<Float>(
                    lumaScale, -2 * blue * (1 - blue) / green * chromaScale,
                    -2 * red * (1 - red) / green * chromaScale),
                SIMD3<Float>(lumaScale, 2 * (1 - blue) * chromaScale, 0),
            ]),
            SIMD3<Float>(fullRange ? 0 : -16.0 / 255.0, -128.0 / 255.0, -128.0 / 255.0)
        )
    }

    func makeOutputColorSpace() -> CGColorSpace? {
        let matrixAttachment =
            switch matrix {
            case .bt601: kCVImageBufferYCbCrMatrix_ITU_R_601_4 as String
            case .bt709: kCVImageBufferYCbCrMatrix_ITU_R_709_2 as String
            case .bt2020: kCVImageBufferYCbCrMatrix_ITU_R_2020 as String
            }
        let attachments =
            [
                kCVImageBufferYCbCrMatrixKey as String: matrixAttachment,
                kCVImageBufferColorPrimariesKey as String: primaries,
                kCVImageBufferTransferFunctionKey as String: transferFunction,
            ] as CFDictionary
        return CVImageBufferCreateColorSpaceFromAttachments(attachments)?.takeRetainedValue()
            ?? CGColorSpace(name: CGColorSpace.itur_709)
    }

    func tracePayload() -> [String: IOSRuntimeTraceValue] {
        [
            "ycbcrMatrix": .string(matrix.rawValue),
            "colorRange": .string(fullRange ? "full" : "video"),
            "colorMatrixSource": .string(matrixSource),
            "colorPrimaries": .string(primaries),
            "transferFunction": .string(transferFunction),
        ]
    }
}
