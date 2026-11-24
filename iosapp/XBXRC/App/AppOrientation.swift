import SwiftUI
import UIKit

@MainActor
enum AppOrientationMode: Equatable {
    case application
    case streaming
}

final class XBXRCAppDelegate: NSObject, UIApplicationDelegate {
    static var orientationMode: AppOrientationMode = .application

    func application(
        _: UIApplication,
        supportedInterfaceOrientationsFor window: UIWindow?
    ) -> UIInterfaceOrientationMask {
        let idiom =
            window?.windowScene?.traitCollection.userInterfaceIdiom
            ?? UIDevice.current.userInterfaceIdiom
        return Self.supportedMask(for: idiom)
    }

    static func supportedMask(for idiom: UIUserInterfaceIdiom) -> UIInterfaceOrientationMask {
        switch orientationMode {
        case .application:
            idiom == .pad ? .all : .allButUpsideDown
        case .streaming:
            idiom == .pad ? .all : .landscape
        }
    }

    static func requestedGeometryMask(
        for idiom: UIUserInterfaceIdiom,
        mode: AppOrientationMode
    ) -> UIInterfaceOrientationMask {
        switch mode {
        case .application:
            idiom == .pad ? .all : .portrait
        case .streaming:
            idiom == .pad ? .all : .landscape
        }
    }
}

@MainActor
final class AppOrientationCoordinator {
    static let shared = AppOrientationCoordinator()

    private var mode: AppOrientationMode = .application

    private init() {}

    func sync(streamingPresented: Bool) {
        let nextMode: AppOrientationMode = streamingPresented ? .streaming : .application
        guard nextMode != mode else { return }
        mode = nextMode
        XBXRCAppDelegate.orientationMode = nextMode
        applyCurrentGeometry()
    }

    func refresh() {
        XBXRCAppDelegate.orientationMode = mode
        applyCurrentGeometry()
    }

    /// 手机编辑器请求系统旋转实际窗口，iPad 方向继续由窗口管理。
    func requestInterfaceOrientation(_ mask: UIInterfaceOrientationMask) {
        guard mode == .application else { return }
        for scene in connectedWindowScenes() {
            guard scene.traitCollection.userInterfaceIdiom == .phone else { continue }
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: mask)) { _ in }
        }
    }

    private func applyCurrentGeometry() {
        let scenes = connectedWindowScenes()
        guard !scenes.isEmpty else { return }
        for scene in scenes {
            for window in scene.windows {
                window.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations()
            }
            // iPad 保持当前窗口几何，由分屏和台前调度管理方向。
            guard scene.traitCollection.userInterfaceIdiom != .pad else { continue }
            let mask = XBXRCAppDelegate.requestedGeometryMask(
                for: scene.traitCollection.userInterfaceIdiom,
                mode: mode
            )
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: mask)) { _ in
            }
        }
    }

    private func connectedWindowScenes() -> [UIWindowScene] {
        let scenes = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .filter { $0.activationState != .unattached }
        let foregroundScenes = scenes.filter {
            $0.activationState == .foregroundActive || $0.activationState == .foregroundInactive
        }
        return foregroundScenes.isEmpty ? scenes : foregroundScenes
    }
}
