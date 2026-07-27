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
        let idiom = window?.windowScene?.traitCollection.userInterfaceIdiom
            ?? UIDevice.current.userInterfaceIdiom
        return Self.supportedMask(for: idiom)
    }

    static func supportedMask(for idiom: UIUserInterfaceIdiom) -> UIInterfaceOrientationMask {
        switch orientationMode {
        case .application:
            idiom == .pad ? .all : .allButUpsideDown
        case .streaming:
            .landscape
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

    private func applyCurrentGeometry() {
        let scenes = connectedWindowScenes()
        guard !scenes.isEmpty else { return }
        for scene in scenes {
            let mask = XBXRCAppDelegate.supportedMask(for: scene.traitCollection.userInterfaceIdiom)
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: mask)) { _ in
            }
            for window in scene.windows {
                window.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations()
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
