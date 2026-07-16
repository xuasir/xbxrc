import SwiftUI

struct XboxLoginView: View {
    let isBusy: Bool
    let errorMessage: String?
    let action: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            Image(systemName: "gamecontroller.fill")
                .font(.system(size: 52, weight: .semibold))
                .foregroundStyle(.green)
                .frame(width: 96, height: 96)
                .glassEffect(.regular, in: Circle())

            VStack(spacing: 8) {
                Text("连接 Xbox")
                    .font(.title2.bold())
                Text("查看个人资料、Gamerscore 与游戏记录")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
            }

            Button(action: action) {
                HStack(spacing: 10) {
                    if isBusy {
                        ProgressView()
                    } else {
                        Image(systemName: "person.badge.key.fill")
                    }
                    Text(isBusy ? "正在连接" : "登录 Xbox")
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(isBusy)
        }
        .padding(32)
        .frame(maxWidth: 460)
    }
}
