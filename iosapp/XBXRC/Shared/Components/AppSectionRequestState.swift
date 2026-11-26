import SwiftUI

/// 无背景的分区提示，整个文字行可点按恢复。
struct AppSectionRequestState: View {
    @Environment(\.locale) private var locale
    @Environment(\.appIsPullRefreshing) private var isPullRefreshing
    let title: String
    let status: SectionRequestStatus
    var identifier: String = "data"
    var showsAuthenticationFailure = false
    var action: (@MainActor () async -> Void)? = nil
    @State private var running = false
    @State private var retryClock = Date()
    @State private var retryTask: Task<Void, Never>?

    var body: some View {
        Group {
            if let failure = status.failure,
                failure.kind != .authentication || showsAuthenticationFailure
            {
                if let action {
                    Button {
                        guard !running && !status.isBusy && !isPullRefreshing else { return }
                        running = true
                        retryTask = Task {
                            await action()
                            running = false
                        }
                    } label: {
                        notice(failure)
                    }
                    .buttonStyle(.plain).disabled(
                        running || status.isBusy || isPullRefreshing || coolingDown(failure)
                    )
                    .accessibilityIdentifier("section-retry-\(identifier)")
                    .accessibilityHint(Text(LocalizedStringKey(retryTitle(failure))))
                } else {
                    notice(failure)
                }
            } else if status.isBusy && status.failure == nil && !isPullRefreshing {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(LocalizedStringKey(status.hasSnapshot ? "正在更新…" : "正在载入…"))
                        .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                }.accessibilityIdentifier("section-loading-\(identifier)")
            }
        }
        .onDisappear {
            retryTask?.cancel()
            retryTask = nil
            running = false
        }
        .task(id: status.failure?.retryAt) {
            retryClock = .now
            if let date = status.failure?.retryAt, date > .now {
                do { try await Task.sleep(for: .seconds(date.timeIntervalSinceNow)) } catch {
                    return
                }
                retryClock = .now
            }
        }
    }

    private func notice(_ failure: SectionRequestFailure) -> some View {
        AppPlainRequestNotice(
            title: appLocalizedFormat(
                status.attempt == .nextPage ? "%@加载更多失败" : status.hasSnapshot ? "%@更新失败" : "%@加载失败",
                locale: locale, appLocalizedString(title, locale: locale)),
            reason: failure.message, detail: snapshotDescription,
            actionTitle: running || status.isBusy ? "正在重试…" : retryTitle(failure),
            showsAction: action != nil, isBusy: (running || status.isBusy) && !isPullRefreshing
        )
        .accessibilityIdentifier("section-error-\(identifier)")
    }

    private var snapshotDescription: String? {
        guard status.hasSnapshot else { return nil }
        if status.attempt == .nextPage { return appLocalizedString("已加载内容可继续查看。", locale: locale) }
        guard let date = status.updatedAt else {
            return appLocalizedString("显示上次内容", locale: locale)
        }
        return appLocalizedFormat(
            "显示上次内容 · %@", locale: locale,
            date.formatted(Date.FormatStyle(date: .omitted, time: .shortened).locale(locale)))
    }
    private func retryTitle(_ failure: SectionRequestFailure) -> String {
        if coolingDown(failure) { return "稍后重试" }
        if failure.kind == .repeatedContinuation || failure.kind == .accessRestricted {
            return failure.actionTitle
        }
        switch status.attempt {
        case .initial: return "重试"
        case .refresh: return "重试更新"
        case .nextPage: return "重试加载更多"
        }
    }
    private func coolingDown(_ failure: SectionRequestFailure) -> Bool {
        failure.retryAt.map { $0 > retryClock } ?? false
    }
}

private struct AppPlainRequestNotice: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .body) private var actionIconSize: CGFloat = 18
    let title: String
    let reason: String
    var detail: String? = nil
    let actionTitle: String
    var showsAction = true
    var isBusy = false
    var body: some View {
        Group {
            if dynamicTypeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 4) {
                    information
                    if showsAction { retryLabel }
                }
            } else {
                HStack(alignment: .center, spacing: 16) {
                    information.frame(maxWidth: .infinity, alignment: .leading)
                    if showsAction { retryLabel }
                }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle()).accessibilityElement(children: .combine)
    }
    private var information: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).appFont(.body, weight: .semibold).foregroundStyle(
                AppThemePalette.textPrimary)
            Text(LocalizedStringKey(reason)).appFont(.caption).foregroundStyle(
                AppThemePalette.textSecondary)
            if let detail {
                Text(detail).appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
            }
        }.fixedSize(horizontal: false, vertical: true)
    }
    private var retryLabel: some View {
        HStack(spacing: 5) {
            Group {
                if isBusy {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: "arrow.clockwise")
                }
            }
            .frame(width: actionIconSize, height: actionIconSize).accessibilityHidden(true)
            Text(LocalizedStringKey(actionTitle))
        }
        .appFont(.body, weight: .semibold).foregroundStyle(AppThemePalette.brand)
        .fixedSize(horizontal: true, vertical: false).frame(minWidth: 44, minHeight: 44)
    }
}

struct AppAccountRequestNotice: View {
    @EnvironmentObject private var auth: AuthStore
    @Environment(\.locale) private var locale
    var body: some View {
        if auth.needsDataSignIn {
            Button {
                Task { await auth.signIn() }
            } label: {
                AppPlainRequestNotice(
                    title: appLocalizedString("登录状态需要更新", locale: locale),
                    reason: "重新登录后继续加载账户资料。", actionTitle: "重新登录", isBusy: auth.isBusy)
            }.buttonStyle(.plain).disabled(auth.isBusy)
                .accessibilityIdentifier("account-request-sign-in")
        }
    }
}
