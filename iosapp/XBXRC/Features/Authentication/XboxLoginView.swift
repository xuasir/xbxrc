import SwiftUI

struct XboxLoginView: View {
    let isBusy: Bool
    let errorMessage: String?
    let action: () -> Void

    var body: some View {
        ZStack {
            Image("LaunchIcon")
                .renderingMode(.original).resizable().scaledToFit()
                .frame(width: 132, height: 132)
                .accessibilityLabel(Text(verbatim: "XBXRC"))
                .accessibilityIdentifier("signed-out-logo")
            VStack(spacing: 16) {
                if let errorMessage {
                    Label(errorMessage, systemImage: "exclamationmark.triangle")
                        .appFont(.caption, weight: .regular).foregroundStyle(.red)
                }
                Button(action: action) {
                    HStack(spacing: 8) {
                        if isBusy { ProgressView().controlSize(.small) }
                        Text(isBusy ? "正在连接" : "登录 Xbox")
                    }
                    .appFont(.body, weight: .medium)
                    .foregroundStyle(AppThemePalette.onBrand)
                    .tint(AppThemePalette.onBrand)
                    .padding(.horizontal, AppLayoutPolicy.pageInset)
                    .frame(maxWidth: .infinity, minHeight: 52)
                    .background(AppThemePalette.brand, in: RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(.plain)
                .opacity(isBusy ? 0.6 : 1)
                .disabled(isBusy)
                .accessibilityLabel(isBusy ? "正在连接 Xbox" : "登录 Xbox")
                .accessibilityIdentifier("signed-out-login")
            }
            .frame(maxWidth: 360)
            .padding(.horizontal, AppLayoutPolicy.pageInset)
            .padding(.bottom, 32)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
