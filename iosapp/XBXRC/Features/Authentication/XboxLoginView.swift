import SwiftUI

struct XboxLoginView: View {
    let isBusy: Bool
    let errorMessage: String?
    let action: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 24)

            VStack(spacing: 26) {
                Image(systemName: "gamecontroller")
                    .font(.system(size: 78, weight: .medium))
                    .foregroundStyle(AppThemePalette.brand)
                    .shadow(
                        color: AppThemePalette.brand.opacity(0.22),
                        radius: 18,
                        y: 8
                    )
                    .accessibilityHidden(true)

                VStack(spacing: 10) {
                    Text("连接 Xbox")
                        .font(.title.bold())
                        .foregroundStyle(.primary)

                    Text("查看个人资料、Gamerscore 与游戏记录")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if let errorMessage {
                    Label {
                        Text(errorMessage)
                            .multilineTextAlignment(.leading)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle")
                    }
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(.red)
                    .frame(maxWidth: 420, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .glassEffect(
                        .regular.tint(.red.opacity(0.1)),
                        in: RoundedRectangle(cornerRadius: 14, style: .continuous)
                    )
                    .accessibilityElement(children: .combine)
                }

                Button(action: action) {
                    HStack(spacing: 10) {
                        if isBusy {
                            ProgressView()
                                .controlSize(.small)
                                .tint(.primary)
                        } else {
                            Image(systemName: "person.badge.key")
                        }

                        Text(isBusy ? "正在连接" : "登录 Xbox")
                    }
                }
                .buttonStyle(XboxLoginButtonStyle())
                .disabled(isBusy)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(isBusy ? "正在连接 Xbox" : "登录 Xbox")
            }
            .frame(maxWidth: 420)

            Spacer(minLength: 24)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct XboxLoginButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline.weight(.semibold))
            .foregroundStyle(.primary)
            .frame(maxWidth: .infinity, minHeight: 54)
            .contentShape(Capsule())
            .glassEffect(
                .regular
                    .interactive()
                    .tint(AppThemePalette.brand.opacity(configuration.isPressed ? 0.26 : 0.18)),
                in: Capsule()
            )
            .scaleEffect(configuration.isPressed ? 0.975 : 1)
            .brightness(configuration.isPressed ? -0.03 : 0)
            .animation(
                reduceMotion ? nil : .snappy(duration: 0.22),
                value: configuration.isPressed
            )
    }
}
