// BEGIN 20261124 Scoped UI style smoke
extension CapabilityMigrationTests {
    @MainActor
    func testScopedUIStyleSmoke() async throws {
        let suite = "XBXRC.UIStyleSmoke.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        for (name, size, theme, language, textSize, selected) in [
            ("styles-phone-dark", CGSize(width: 390, height: 844), ColorScheme.dark,
             "zh-Hans", DynamicTypeSize.large, false),
            ("styles-phone-light", CGSize(width: 390, height: 844), ColorScheme.light,
             "zh-Hans", DynamicTypeSize.large, true),
            ("styles-narrow-accessibility", CGSize(width: 320, height: 1_000), ColorScheme.dark,
             "en", DynamicTypeSize.accessibility3, true),
            ("styles-tablet-light", CGSize(width: 1_024, height: 768), ColorScheme.light,
             "en", DynamicTypeSize.large, false),
        ] {
            let root = ScopedUIStyleShowcase(selected: selected)
                .environmentObject(settings)
                .environment(\.colorScheme, theme).preferredColorScheme(theme)
                .environment(\.locale, Locale(identifier: language))
                .environment(\.dynamicTypeSize, textSize)
                .transaction { $0.disablesAnimations = true }
                .appInformationTypography()
            try await attachLayoutSnapshot(root: root, size: size, name: name)
        }
    }
}

private struct ScopedUIStyleShowcase: View {
    @State private var touchOnly: Bool
    @State private var tab = "概览"
    @State private var group = 0
    private let groups = ["网络", "显示", "恢复", "会话"]

    init(selected: Bool) { _touchOnly = State(initialValue: selected) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("图标与文字").appFont(.section, weight: .semibold)
                Label {
                    Text("正在游玩 Halo Infinite").appFont(.bodySmall)
                } icon: {
                    AppIconView(.controller, size: 14)
                }
                .labelStyle(AppIconLabelStyle())
                AppInlineHint("旋转设备或调整窗口以切换方向")
                AppNavigationRow(
                    title: "触屏手柄", subtitle: "布局 · 大小 · 透明度", icon: .touch,
                    compact: true)
                Text("筛选与工具").appFont(.section, weight: .semibold)
                AppContentToolbar {
                    AppSegmentedFilter(
                        selection: $touchOnly, values: [false, true],
                        title: { $0 ? "官方触控" : "全部" },
                        identifier: "style-input-filter", label: "控制方式", fillsWidth: false)
                    AppInformationPicker("排序", selection: .constant("recent"), selectedTitle: "最近游玩") {
                        Text("最近游玩").tag("recent")
                    }
                    Button("重置筛选") { touchOnly = false }
                        .buttonStyle(AppContentCapsuleButtonStyle())
                }
                AppSegmentedFilter(
                    selection: $group, values: Array(groups.indices),
                    title: { groups[$0] }, identifier: "style-diagnostic-groups", label: "诊断分组")
                HStack {
                    AppSegmentedFilter(
                        selection: .constant(TouchLayoutOrientation.portrait),
                        values: TouchLayoutOrientation.allCases, title: { $0.title },
                        identifier: "style-orientation", label: "预览方向", fillsWidth: false)
                    Spacer(minLength: 0)
                    Button("保存") {}.buttonStyle(AppContentCapsuleButtonStyle())
                }
                Text("内容分类").appFont(.section, weight: .semibold)
                AppTextTabs(
                    selection: $tab, values: ["概览", "成就", "Xbox 相册", "游玩统计"],
                    title: { $0 }, identifier: "style-content-tabs", inset: 0)
            }
            .foregroundStyle(AppThemePalette.textPrimary)
            .padding(AppLayoutPolicy.pageInset)
        }
    }
}
// END 20261124 Scoped UI style smoke
