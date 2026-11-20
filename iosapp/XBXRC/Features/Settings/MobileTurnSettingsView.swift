import SwiftUI
import UniformTypeIdentifiers

struct MobileTurnSettingsView: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @State private var importing = false
    @State private var draft: MobileTurnConfiguration?
    @State private var error: String?
    @State private var confirmingClear = false

    var body: some View {
        AppSettingsForm {
            Section {
                AppSettingsToggle("使用自定义中继", isOn: $settings.customTurnEnabled)
                    .disabled(settings.customTurn == nil)
                    .appSettingsHint(
                        settings.customTurn == nil ? "导入服务器配置后可启用自定义中继。" : "下次连接生效；凭据安全保存在本机。",
                        identifier: "turn-connection")
                if let config = settings.customTurn {
                    AppFactGroup(facts: [("服务器", config.serverURL), ("凭据", "已安全保存")])
                        .appSettingsInformation()
                }
            } header: {
                Text("连接")
            }
            Section {
                AppSettingsActionButton("从文件导入") { importing = true }
                    .appSettingsHint("选择 JSON 配置，核对服务器后应用", identifier: "turn-format")
                PasteButton(payloadType: String.self) { values in
                    guard let text = values.first else { return }
                    preview(Data(text.utf8))
                }
                .labelStyle(.titleAndIcon)
                .buttonStyle(AppSettingsRowButtonStyle(inset: AppLayoutPolicy.rowInset))
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .containerValue(\.appSettingsRowOwnsPadding, true)
                AppSettingsActionButton("复制配置模板") {
                    UIPasteboard.general.string = """
                        {"server_url":"turn:relay.example.com:3478?transport=udp","server_username":"user","server_credential":"password"}
                        """
                }
            } header: {
                Text("导入配置")
            } footer: {
                AppInstructionSteps(steps: ["导入配置", "核对服务器", "确认应用"])
            }
            if settings.customTurn != nil {
                Section {
                    AppSettingsActionButton("移除自定义中继", role: .destructive) {
                        confirmingClear = true
                    }
                }
            }
            if let error {
                Section {
                    Text(LocalizedStringKey(error)).foregroundStyle(.red).appSettingsInformation()
                }
            }
        }
        .appPageToolbar("自定义 TURN 中继")
        .fileImporter(isPresented: $importing, allowedContentTypes: [.json, .plainText]) { result in
            do {
                let url = try result.get()
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let handle = try FileHandle(forReadingFrom: url)
                defer { try? handle.close() }
                preview(try handle.read(upToCount: 16_385) ?? Data())
            } catch { self.error = "配置文件读取失败，请重新选择。" }
        }
        .alert(
            "应用中继配置？",
            isPresented: Binding(
                get: { draft != nil }, set: { if !$0 { draft = nil } }
            )
        ) {
            Button("取消", role: .cancel) { draft = nil }
            Button("应用") {
                guard let draft else { return }
                do {
                    try settings.applyCustomTurn(draft)
                    error = nil
                } catch { self.error = "配置保存失败，请重试。" }
                self.draft = nil
            }
        } message: {
            Text(draft?.serverURL ?? "")
        }
        .confirmationDialog(
            "移除已保存的中继配置？", isPresented: $confirmingClear,
            titleVisibility: .visible
        ) {
            Button("移除", role: .destructive) {
                do {
                    try settings.clearCustomTurn()
                    error = nil
                } catch { self.error = "配置移除失败，请重试。" }
            }
        }
    }

    private func preview(_ data: Data) {
        do {
            draft = try MobileTurnConfiguration.imported(from: data)
            error = nil
        } catch { self.error = MobileTurnImportError.invalid.localizedDescription }
    }
}
