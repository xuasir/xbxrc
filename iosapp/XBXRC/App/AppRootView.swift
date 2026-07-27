import SwiftUI

enum AppThemePalette {
    static let brand = Color(red: 16 / 255, green: 124 / 255, blue: 16 / 255)

    static let canvas = adaptive(
        dark: UIColor(red: 15 / 255, green: 15 / 255, blue: 16 / 255, alpha: 1),
        light: UIColor(red: 244 / 255, green: 247 / 255, blue: 245 / 255, alpha: 1)
    )
    static let canvasTop = adaptive(
        dark: UIColor(red: 13 / 255, green: 27 / 255, blue: 18 / 255, alpha: 1),
        light: UIColor(red: 232 / 255, green: 243 / 255, blue: 235 / 255, alpha: 1)
    )
    static let canvasBottom = adaptive(
        dark: UIColor(red: 18 / 255, green: 22 / 255, blue: 30 / 255, alpha: 1),
        light: UIColor(red: 235 / 255, green: 241 / 255, blue: 248 / 255, alpha: 1)
    )
    static let brandAura = adaptive(
        dark: UIColor(red: 16 / 255, green: 124 / 255, blue: 16 / 255, alpha: 0.22),
        light: UIColor(red: 43 / 255, green: 194 / 255, blue: 74 / 255, alpha: 0.16)
    )
    static let coolAura = adaptive(
        dark: UIColor(red: 55 / 255, green: 99 / 255, blue: 118 / 255, alpha: 0.18),
        light: UIColor(red: 129 / 255, green: 172 / 255, blue: 196 / 255, alpha: 0.14)
    )

    private static func adaptive(dark: UIColor, light: UIColor) -> Color {
        Color(uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark ? dark : light
        })
    }
}

struct AppThemeBackground: View {
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        GeometryReader { geometry in
            let radius = max(geometry.size.width, geometry.size.height)
            let themeMarkSize = min(radius * 0.82, geometry.size.width * 1.5)
            let themeMarkOpacity = colorScheme == .dark ? 0.035 : 0.1
            let themeMarkBlur = colorScheme == .dark ? 1.2 : 0.6

            ZStack {
                LinearGradient(
                    colors: [
                        AppThemePalette.canvasTop,
                        AppThemePalette.canvas,
                        AppThemePalette.canvasBottom,
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )

                RadialGradient(
                    colors: [AppThemePalette.brandAura, .clear],
                    center: UnitPoint(x: 0.12, y: 0.08),
                    startRadius: 0,
                    endRadius: radius * 0.72
                )

                RadialGradient(
                    colors: [AppThemePalette.coolAura, .clear],
                    center: UnitPoint(x: 0.92, y: 0.86),
                    startRadius: 0,
                    endRadius: radius * 0.64
                )

                Image("LaunchIcon")
                    .resizable()
                    .scaledToFit()
                    .frame(width: themeMarkSize, height: themeMarkSize)
                    .opacity(themeMarkOpacity)
                    .rotationEffect(.degrees(-10))
                    .offset(
                        x: geometry.size.width * 0.2,
                        y: -geometry.size.height * 0.06
                    )
                    .blur(radius: themeMarkBlur)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

extension View {
    func appThemeCanvas() -> some View {
        frame(maxWidth: .infinity, maxHeight: .infinity)
            .background {
                AppThemeBackground()
            }
    }
}

struct AppRootView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var streamingStore: StreamingFeatureStore
    @State private var selection: AppSection = .library
    @State private var launchVisible = true

    var body: some View {
        ZStack {
            if streamingStore.isPresentingPlayer {
                StreamingPageRoot(store: streamingStore)
            } else {
                AppThemeBackground()

                TabView(selection: $selection) {
                    GameLibraryView(isActive: selection == .library)
                        .tabItem {
                            Label("游戏库", systemImage: "rectangle.stack.fill")
                        }
                        .tag(AppSection.library)

                    HostListView(isActive: selection == .hosts)
                        .tabItem {
                            Label("主机", systemImage: "desktopcomputer")
                        }
                        .tag(AppSection.hosts)

                    AchievementsView(isActive: selection == .achievements)
                        .tabItem {
                            Label("成就", systemImage: "trophy.fill")
                        }
                        .tag(AppSection.achievements)

                    ProfileView(isActive: selection == .my)
                        .tabItem {
                            Label("我的", systemImage: "person.crop.circle.fill")
                        }
                        .tag(AppSection.my)
                }
                .tint(AppThemePalette.brand)
            }

            if launchVisible && !streamingStore.isPresentingPlayer {
                LaunchExperienceView(isRestoring: authStore.phase == .restoring) {
                    launchVisible = false
                }
            }
        }
        .onAppear {
            AppOrientationCoordinator.shared.sync(
                streamingPresented: streamingStore.isPresentingPlayer
            )
        }
        .onChange(of: streamingStore.isPresentingPlayer) { _, isPresenting in
            AppOrientationCoordinator.shared.sync(streamingPresented: isPresenting)
        }
    }
}

private enum AppSection: Hashable {
    case library
    case hosts
    case achievements
    case my
}

private struct StreamingPageRoot: View {
    @ObservedObject var store: StreamingFeatureStore

    var body: some View {
        StreamingPlayerView(store: store)
            .ignoresSafeArea()
    }
}
