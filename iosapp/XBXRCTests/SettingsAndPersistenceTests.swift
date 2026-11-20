import Foundation
import GameController
import Metal
import XCTest

@testable import XBXRC

final class SettingsAndPersistenceTests: XCTestCase {
    func testQualityCapClampsH264HintsAndRemoteCapabilityUsesActiveVideo() {
        let plan = StreamingWebRtcPlan.desktopWebRtcDirect.withVideoBitrateCap(2000)
        XCTAssertEqual(plan.maxVideoBitrateKbps, 2000)
        XCTAssertEqual(plan.minVideoBitrateKbps, 2000)
        XCTAssertEqual(plan.startVideoBitrateKbps, 2000)
        let sdp =
            "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 profile-level-id=640032\r\n"
        let offer = StreamingOfferSdpProjector().project(sdp, plan: plan)
        XCTAssertTrue(offer.contains("b=AS:2000"))
        XCTAssertTrue(offer.contains("x-google-min-bitrate=2000"))
        XCTAssertTrue(offer.contains("x-google-start-bitrate=2000"))
        for (bandwidth, expected) in [
            ("b=AS:15000", 15000), ("b=TIAS:12000000", 12000),
            ("a=fmtp:96 x-google-max-bitrate=11000", 11000),
        ] {
            let answer =
                "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nb=AS:128\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=sendonly\r\n\(bandwidth)\r\n"
            XCTAssertEqual(StreamingOfferSdpProjector.videoBitrateCapKbps(in: answer), expected)
            for direction in ["a=recvonly", "a=inactive"] {
                XCTAssertNil(
                    StreamingOfferSdpProjector.videoBitrateCapKbps(
                        in: answer.replacingOccurrences(of: "a=sendonly", with: direction)))
            }
            XCTAssertNil(
                StreamingOfferSdpProjector.videoBitrateCapKbps(
                    in: answer.replacingOccurrences(of: "m=video 9", with: "m=video 0")))
        }
    }

    func testFrameIntervalUsesPresentFactsAndIgnoresFreezeGap() {
        var interval = StreamingFrameIntervalEstimator()
        interval.observe(100)
        XCTAssertNil(interval.intervalMilliseconds)
        interval.observe(116)
        XCTAssertEqual(interval.intervalMilliseconds, 16)
        interval.observe(136)
        XCTAssertEqual(interval.intervalMilliseconds ?? 0, 16.8, accuracy: 0.001)
        interval.observe(6000)
        interval.observe(.nan)
        XCTAssertEqual(interval.intervalMilliseconds ?? 0, 16.8, accuracy: 0.001)
    }

    @MainActor
    func testEveryResolutionCodecAndBitrateBoundaryRestoresIntoNextSession() throws {
        let suite = "XBXRC.StreamSettingMatrix.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = AppSettingsStore(defaults: defaults)
        for resolution in StreamingResolutionOption.allCases {
            store.cloudResolution = resolution
            store.homeResolution = resolution
            let restored = AppSettingsStore(defaults: defaults).streamingSessionSettings
            XCTAssertEqual(restored.cloudResolution, resolution.rawValue)
            XCTAssertEqual(restored.homeResolution, resolution.rawValue)
        }
        for codec in StreamingCodecOption.allCases {
            store.codecPreference = codec
            XCTAssertEqual(
                AppSettingsStore(defaults: defaults).streamingSessionSettings.videoCodec,
                codec.rawValue)
        }
        for mode in StreamingBitrateMode.allCases {
            for boundary in [0, 1, 20, 200] {
                store.homeBitrateMode = mode
                store.cloudBitrateMode = mode
                store.audioBitrateMode = mode
                store.homeBitrateMbps = boundary
                store.cloudBitrateMbps = boundary
                store.audioBitrateKbps = boundary
                let restored = AppSettingsStore(defaults: defaults).streamingSessionSettings
                XCTAssertEqual(restored.homeBitrateMode, mode.rawValue)
                XCTAssertEqual(restored.cloudBitrateMode, mode.rawValue)
                XCTAssertEqual(restored.audioBitrateMode, mode.rawValue)
                XCTAssertEqual(restored.homeBitrateMbps, boundary)
                XCTAssertEqual(restored.cloudBitrateMbps, boundary)
                XCTAssertEqual(restored.audioBitrateKbps, boundary)
            }
        }
        for enabled in [false, true] {
            store.preferIPv6 = enabled
            store.homeTurnFallbackEnabled = enabled
            store.mobile.startMuted = enabled
            store.mobile.keepScreenAwake = enabled
            store.mobile.enableAudioControl = enabled
            store.mobile.preventResolutionDrops = enabled
            store.mobile.powerOn = enabled
            let restored = AppSettingsStore(defaults: defaults).streamingSessionSettings
            XCTAssertEqual(restored.preferIPv6, enabled)
            XCTAssertEqual(restored.homeTurnFallback, enabled)
            XCTAssertEqual(restored.startMuted, enabled)
            XCTAssertEqual(restored.keepScreenAwake, enabled)
            XCTAssertEqual(restored.enableAudioControl, enabled)
            XCTAssertEqual(restored.preventResolutionDrops, enabled)
            XCTAssertEqual(restored.powerOn, enabled)
        }
    }

    @MainActor
    func testLegacyInputPreferencesKeepMobileControlsAndRetireDesktopFields() throws {
        let legacy = Data(
            #"{"inputSource":"keyboard","keyboardMapping":{"bindings":[{"key":"keyJ","control":"south"}]},"touchControllerMode":"always","touchOpacity":0.4,"invertLeftY":true}"#
                .utf8)
        let preferences = try JSONDecoder().decode(MobilePreferences.self, from: legacy)
        XCTAssertEqual(preferences.touchControllerMode, .always)
        XCTAssertEqual(preferences.touchOpacity, 0.4)
        XCTAssertTrue(preferences.invertLeftY)
        let encoded = try JSONEncoder().encode(preferences)
        let persisted = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertNil(persisted["inputSource"])
        XCTAssertNil(persisted["keyboardMapping"])
        let input = StreamInputSession(preferences: { preferences }, controller: { nil })
        XCTAssertEqual(input.outputState(now: 0), .neutral)
        input.setTouchButton(16, pressed: true, id: "A")
        XCTAssertEqual(input.outputState(now: 1).buttonMask, 16)
        XCTAssertEqual(input.source, .touch)
        input.setTouchButton(16, pressed: false, id: "A")
        XCTAssertEqual(input.outputState(now: 2), .neutral)
    }

    @MainActor
    func testTouchAndExternalControllerShareOwnershipAndReleaseDuringHandoff() throws {
        let controller = GCController.withExtendedGamepad()
        let pad = try XCTUnwrap(controller.extendedGamepad)
        var selected: GCController?
        let input = StreamInputSession(controller: { selected })
        XCTAssertEqual(input.outputState(now: 0), .neutral)
        XCTAssertEqual(input.source, .idle)
        input.setTouchButton(16, pressed: true, id: "A")
        XCTAssertEqual(input.outputState(now: 1).buttonMask, 16)
        XCTAssertEqual(input.source, .touch)
        selected = controller
        XCTAssertEqual(input.outputState(now: 2), .neutral)
        pad.buttonB.setValue(1)
        XCTAssertEqual(input.outputState(now: 3).buttonMask, 32)
        XCTAssertEqual(input.source, .controller)
        input.setTouchButton(16, pressed: true, id: "A")
        XCTAssertEqual(input.outputState(now: 4).buttonMask, 32)
        pad.buttonB.setValue(0)
        XCTAssertEqual(input.outputState(now: 5).buttonMask, 16)
        XCTAssertEqual(input.source, .touch)
        input.setSuppressed(true, reason: .menu)
        XCTAssertEqual(input.outputState(now: 6), .neutral)
        input.setSuppressed(false, reason: .menu)
        XCTAssertEqual(input.outputState(now: 7), .neutral)
    }

    @MainActor
    func testRetiredInputPreferencesRestoreGamepadAndStandardLayout() throws {
        let suite = "XBXRC.TouchLayoutMigration.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        for layoutID in ["classic"] + (try TouchLayoutCatalog.builtIn()).map(\.id) {
            var preferences = MobilePreferences()
            preferences.touchLayoutID = layoutID
            preferences.touchControllerMode = .always
            preferences.touchOpacity = 0.4
            preferences.invertLeftY = true
            var persisted = try XCTUnwrap(
                JSONSerialization.jsonObject(with: JSONEncoder().encode(preferences))
                    as? [String: Any])
            persisted["inputSource"] = "keyboard"
            persisted["keyboardMapping"] = ["bindings": [["key": "keyJ", "control": "south"]]]
            defaults.set(
                try JSONSerialization.data(withJSONObject: persisted),
                forKey: MobilePreferences.persistenceKey)

            let store = AppSettingsStore(defaults: defaults)
            XCTAssertEqual(store.mobile.touchLayoutID, "classic", layoutID)
            XCTAssertEqual(store.mobile.touchControllerMode, .always)
            XCTAssertEqual(store.mobile.touchOpacity, 0.4)
            XCTAssertTrue(store.mobile.invertLeftY)

            store.mobile.touchLayoutID = layoutID
            store.mobile.touchOpacity = 0.5
            let restored = AppSettingsStore(defaults: defaults)
            XCTAssertEqual(restored.mobile.touchLayoutID, "classic", layoutID)
            XCTAssertEqual(restored.mobile.touchControllerMode, .always)
            XCTAssertEqual(restored.mobile.touchOpacity, 0.5)
            XCTAssertTrue(restored.mobile.invertLeftY)
            let saved = try XCTUnwrap(defaults.data(forKey: MobilePreferences.persistenceKey))
            let savedFields = try XCTUnwrap(
                JSONSerialization.jsonObject(with: saved) as? [String: Any])
            XCTAssertNil(savedFields["inputSource"])
            XCTAssertNil(savedFields["keyboardMapping"])
        }
    }

    @MainActor
    func testDataTouchLayoutUsesTheVirtualGamepadAndReleasesOnMenu() {
        let input = StreamInputSession(controller: { nil })
        XCTAssertEqual(input.outputState(now: 0), .neutral)
        var state = IOSStreamGamepadState.neutral
        state.buttonMask = 16
        state.rightTrigger = 0.5
        input.setTouchLayoutState(state, active: true)
        XCTAssertEqual(input.outputState(now: 1), state)
        XCTAssertEqual(input.source, .touch)
        input.setSuppressed(true, reason: .menu)
        input.setTouchLayoutState(state, active: true)
        XCTAssertEqual(input.outputState(now: 2), .neutral)
        input.setSuppressed(false, reason: .menu)
        XCTAssertEqual(input.outputState(now: 3), .neutral)
        input.setTouchLayoutState(state, active: true)
        XCTAssertEqual(input.outputState(now: 4), state)
        input.setTouchLayoutState(.neutral, active: false)
        XCTAssertEqual(input.outputState(now: 5), .neutral)
    }

    @MainActor
    func testGamepadInputAndRumbleUseTheSameSelectedDeviceAndLogicalSlot() throws {
        let first = GCController.withExtendedGamepad()
        let second = GCController.withExtendedGamepad()
        var selected = second
        let input = StreamInputSession(controller: { selected })
        let channels = XboxStreamDataChannels(inputSession: input, eventSink: { _, _ in })
        XCTAssertTrue(input.currentController === second)
        XCTAssertTrue(channels.gamepadController(for: nil) === second)
        XCTAssertTrue(channels.gamepadController(for: 0) === second)
        XCTAssertNil(channels.gamepadController(for: 1))
        selected = first
        XCTAssertTrue(channels.gamepadController(for: 0) === first)
        XCTAssertTrue(input.currentController === first)
    }

    @MainActor
    func testPowerOnPreferencePreservesLegacyValuesAndReachesTheNextSession() throws {
        let legacy = try JSONDecoder().decode(
            MobilePreferences.self,
            from: Data(#"{"startMuted":true,"preventResolutionDrops":true}"#.utf8))
        XCTAssertTrue(legacy.startMuted)
        XCTAssertTrue(legacy.preventResolutionDrops)
        XCTAssertFalse(legacy.powerOn)
        let suite = "XBXRC.StreamPowerOn.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        for enabled in [true, false] {
            settings.mobile.powerOn = enabled
            let restored = AppSettingsStore(defaults: defaults)
            XCTAssertEqual(restored.mobile.powerOn, enabled)
            XCTAssertEqual(restored.streamingSessionSettings.powerOn, enabled)
        }
    }

    func testNativeIceProjectionKeepsBindingAndSharedSource() throws {
        let candidate = StreamingIceCandidate(
            sdp: "a=candidate:1 1 UDP 123 192.0.2.1 5000 typ host",
            sdpMid: "video", sdpMLineIndex: 1, usernameFragment: "current-ufrag",
            source: "session-configuration")
        let native = try XCTUnwrap(StreamingNativeIceCandidateAdapter.adapt(candidate))
        XCTAssertEqual(native.sdpMid, "video")
        XCTAssertEqual(native.sdpMLineIndex, 1)
        XCTAssertEqual(native.usernameFragment, "current-ufrag")
        XCTAssertEqual(native.source, "session-configuration")
        XCTAssertEqual(native.sdp, "candidate:1 1 UDP 123 192.0.2.1 5000 typ host")
    }

    func testControllerHistoryRecordsPressEdgesAndKeepsLastTenInputs() {
        var history = MobileControllerInputHistory()
        var pressed = IOSStreamGamepadState.neutral
        pressed.buttonMask = 16
        history.sample(pressed)
        history.sample(pressed)
        XCTAssertEqual(history.entries, [.a])
        for _ in 0..<12 {
            history.sample(.neutral)
            pressed.buttonMask = 32
            history.sample(pressed)
        }
        XCTAssertEqual(history.entries, Array(repeating: .b, count: 10))
        history.reset()
        XCTAssertTrue(history.entries.isEmpty)
        history.sample(pressed)
        XCTAssertEqual(history.entries, [.b])
    }

    func testLegacyMenuDurationUsesTheFixedTriggerThreshold() throws {
        let old = try JSONDecoder().decode(
            MobilePreferences.self, from: Data(#"{"menuHoldDuration":2}"#.utf8))
        XCTAssertEqual(old.normalized().menuHoldDuration, 0.6)
    }

    @MainActor
    func testChoosingMappingTargetImmediatelyCapturesAndReleasesInput() throws {
        let suite = "XBXRC.DirectCapture.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let controller = GCController.withExtendedGamepad()
        let pad = try XCTUnwrap(controller.extendedGamepad)
        let input = StreamInputSession(preferences: { settings.mobile }, controller: { controller })
        let capture = MobileControllerCaptureSession()
        capture.begin(.control("a"), input: input)
        XCTAssertEqual(capture.target, .control("a"))
        XCTAssertTrue(input.isSuppressed)
        capture.poll(input: input, settings: settings)
        pad.buttonB.setValue(1)
        capture.poll(input: input, settings: settings)
        pad.buttonB.setValue(0)
        capture.poll(input: input, settings: settings)
        XCTAssertNil(capture.target)
        XCTAssertFalse(input.isSuppressed)
        XCTAssertEqual(
            settings.mobile.gamepadBindings[input.controllerLayoutKey]?["a"], .buttons([.b]))
        capture.begin(.control("x"), input: input)
        capture.cancel()
        XCTAssertFalse(input.isSuppressed)
        XCTAssertNil(capture.target)
    }

    @MainActor
    func testSettingsMigrationNormalizesAndWritesBackInvalidLegacyValues() throws {
        let suiteName = "XBXRC.SettingsTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }

        defaults.set("unknown", forKey: AppSettingsStore.appearanceModeKey)
        defaults.set("zh_CN", forKey: AppSettingsStore.preferredGameLocaleKey)
        defaults.set(999, forKey: AppSettingsStore.cloudBitrateMbpsKey)
        defaults.set(-10, forKey: AppSettingsStore.audioBitrateKbpsKey)

        let store = AppSettingsStore(defaults: defaults)

        XCTAssertEqual(store.appearanceMode, .system)
        XCTAssertEqual(store.preferredGameLocale, "zh-CN")
        XCTAssertEqual(store.cloudBitrateMbps, 200)
        XCTAssertEqual(store.audioBitrateKbps, 0)
        XCTAssertEqual(defaults.string(forKey: AppSettingsStore.appearanceModeKey), "system")
        XCTAssertEqual(defaults.string(forKey: AppSettingsStore.preferredGameLocaleKey), "zh-CN")
        XCTAssertEqual(defaults.integer(forKey: AppSettingsStore.cloudBitrateMbpsKey), 200)
        XCTAssertNotNil(defaults.data(forKey: AppSettingsPersistence.snapshotKey))
    }

    @MainActor
    func testSettingsExposePCBoundsAndPreserveDefaultLanguageInStreamSnapshot() throws {
        let suite = "XBXRC.Parity.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(
            StreamingResolutionOption.homeChoices.map(\.rawValue), [1440, 1081, 1080, 720])
        store.homeResolution = .p1440
        store.homeBitrateMode = .custom
        store.cloudBitrateMode = .custom
        store.audioBitrateMode = .custom
        store.homeBitrateMbps = 0
        store.cloudBitrateMbps = 0
        store.audioBitrateKbps = 0
        store.preferredGameLocale = ""
        let snapshot = store.streamingSessionSettings
        XCTAssertEqual(snapshot.homeResolution, 1440)
        XCTAssertEqual(snapshot.homeBitrateMbps, 0)
        XCTAssertEqual(snapshot.cloudBitrateMbps, 0)
        XCTAssertEqual(snapshot.audioBitrateKbps, 0)
        XCTAssertEqual(snapshot.preferredGameLocale, "")
        store.audioBitrateKbps = 999
        XCTAssertEqual(AppSettingsStore(defaults: defaults).audioBitrateKbps, 200)
        for ratio in ["ratio18x9", "ratio21x9"] {
            store.mobile.videoFit = ratio
            XCTAssertEqual(AppSettingsStore(defaults: defaults).mobile.videoFit, ratio)
        }
        store.mobile.compactPerformanceOverlay = true
        XCTAssertTrue(AppSettingsStore(defaults: defaults).mobile.compactPerformanceOverlay)
    }

    func testPhysicalCaptureIgnoresStartAAndKeepsMaximumSimultaneousChord() {
        var capture = MobileRawPhysicalCapture()
        XCTAssertNil(capture.sample(buttons: ["a": 1], axes: [:]))
        XCTAssertNil(capture.sample(buttons: [:], axes: [:]))
        XCTAssertNil(capture.sample(buttons: ["a": 1, "b": 1], axes: [:]))
        XCTAssertNil(capture.sample(buttons: ["b": 1, "x": 1], axes: [:]))
        XCTAssertNil(capture.sample(buttons: ["x": 1], axes: [:]))
        XCTAssertEqual(capture.sample(buttons: [:], axes: [:]), .buttons([.a, .b]))
        var axes = MobileRawPhysicalCapture(includesAxes: true, axisOnly: true)
        XCTAssertNil(axes.sample(buttons: [:], axes: ["rightX": 0]))
        XCTAssertNil(axes.sample(buttons: ["b": 1], axes: ["rightX": -0.8]))
        XCTAssertNil(axes.sample(buttons: [:], axes: ["rightX": -0.2]))
        XCTAssertEqual(
            axes.sample(buttons: [:], axes: ["rightX": 0]), .axis(.rightX, inverted: true))
        var idleTrigger = MobileRawPhysicalCapture(
            rests: ["axis:trigger": -1], includesAxes: true, axisOnly: true)
        XCTAssertNil(idleTrigger.sample(buttons: [:], axes: ["leftX": 0, "axis:trigger": -1]))
        XCTAssertNil(idleTrigger.sample(buttons: [:], axes: ["leftX": 0.8, "axis:trigger": -1]))
        XCTAssertEqual(
            idleTrigger.sample(buttons: [:], axes: ["leftX": 0, "axis:trigger": -1]),
            .axis(.leftX, inverted: false))
    }

    @MainActor
    func testRemappedShortcutsRecordAndTriggerTheSameXboxProjection() throws {
        let suite = "XBXRC.MappedShortcuts.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        let controller = GCController.withExtendedGamepad()
        let pad = try XCTUnwrap(controller.extendedGamepad)
        let input = StreamInputSession(preferences: { settings.mobile }, controller: { controller })
        var bindings: [String: MobileControlBinding] = [:]
        MobileControlBinding.assign(.buttons([.b]), target: "a", to: &bindings)
        settings.mobile.gamepadBindings[input.controllerLayoutKey] = bindings
        let capture = MobileControllerCaptureSession()
        pad.buttonA.setValue(1)
        capture.begin(.shortcut(.menu), input: input)
        capture.poll(input: input, settings: settings)
        pad.buttonA.setValue(0)
        capture.poll(input: input, settings: settings)
        pad.buttonB.setValue(1)
        pad.rightTrigger.setValue(1)
        capture.poll(input: input, settings: settings)
        pad.buttonB.setValue(0)
        pad.rightTrigger.setValue(0)
        capture.poll(input: input, settings: settings)
        XCTAssertNil(capture.target)
        XCTAssertEqual(settings.mobile.shortcutButtons(for: .menu), [.a, .rightTrigger])
        XCTAssertEqual(input.outputState(now: 0), .neutral)
        pad.buttonB.setValue(1)
        pad.rightTrigger.setValue(1)
        XCTAssertEqual(input.outputState(now: 100), .neutral)
        XCTAssertEqual(input.outputState(now: 800), .neutral)
        XCTAssertEqual(input.menuRevealToken, 1)
        _ = input.outputState(now: 1_800)
        XCTAssertEqual(input.menuRevealToken, 1)
        pad.buttonB.setValue(0)
        pad.rightTrigger.setValue(0)
        _ = input.outputState(now: 1_900)
        pad.buttonA.setValue(1)
        XCTAssertEqual(input.outputState(now: 2_000).buttonMask, 32)
        XCTAssertEqual(input.menuRevealToken, 1)
    }

    func testMappingSwapsSingleBindingsAndAbsorbsChordComponents() {
        var bindings: [String: MobileControlBinding] = [:]
        MobileControlBinding.assign(.buttons([.b]), target: "a", to: &bindings)
        let swapped = MobileControlBinding.apply(
            IOSStreamGamepadState(buttonMask: 32), bindings: bindings)
        XCTAssertEqual(swapped.buttonMask, 16)
        XCTAssertEqual(
            MobileControlBinding.apply(IOSStreamGamepadState(buttonMask: 16), bindings: bindings)
                .buttonMask, 32)
        bindings = ["x": .buttons([.a, .b])]
        XCTAssertEqual(
            MobileControlBinding.apply(
                IOSStreamGamepadState(buttonMask: 16 | 32), bindings: bindings
            ).buttonMask, 64)
        XCTAssertEqual(
            MobileControlBinding.apply(IOSStreamGamepadState(buttonMask: 16), bindings: bindings)
                .buttonMask, 16)
        bindings = [:]
        MobileControlBinding.assign(.axis(.rightX, inverted: true), target: "leftX", to: &bindings)
        let state = MobileControlBinding.apply(
            IOSStreamGamepadState(leftThumbX: 0.3, rightThumbX: 0.8), bindings: bindings)
        XCTAssertEqual(state.leftThumbX, -0.8)
        XCTAssertEqual(state.rightThumbX, 0.3)
    }

    func testAdditionalButtonsAndAxesSupportAliasesAndFullRangeTriggers() {
        var reader = MobileRawPhysicalCapture(rests: ["axis:trigger": -1], includesAxes: true)
        XCTAssertNil(reader.sample(buttons: ["a": 1], axes: ["axis:trigger": -1]))
        XCTAssertNil(reader.sample(buttons: [:], axes: ["axis:trigger": -1]))
        XCTAssertNil(reader.sample(buttons: ["extra:paddle": 1], axes: ["axis:trigger": -1]))
        XCTAssertEqual(
            reader.sample(buttons: [:], axes: ["axis:trigger": -1]),
            .rawButtons(["extra:paddle"], any: false))
        reader = MobileRawPhysicalCapture(rests: ["axis:trigger": -1], includesAxes: true)
        XCTAssertNil(reader.sample(buttons: [:], axes: ["axis:trigger": -1]))
        XCTAssertNil(reader.sample(buttons: [:], axes: ["axis:trigger": 0.8]))
        XCTAssertEqual(
            reader.sample(buttons: [:], axes: ["axis:trigger": -1]),
            .rawAxis("axis:trigger", inverted: false, fullRange: true))
        let bindings: [String: MobileControlBinding] = [
            "a": .rawButtons(["a", "extra:paddle"], any: true),
            "leftTrigger": .rawAxis("axis:trigger", inverted: false, fullRange: true),
        ]
        let value = MobileControlBinding.apply(
            .neutral, bindings: bindings, buttons: ["extra:paddle": 1], axes: ["axis:trigger": 0.5])
        XCTAssertEqual(value.buttonMask, 16)
        XCTAssertEqual(value.leftTrigger, 0.75)
        XCTAssertEqual(MobileControlBinding.apply(.neutral, bindings: bindings).leftTrigger, 0)
        XCTAssertEqual(
            MobileControlBinding.apply(IOSStreamGamepadState(buttonMask: 16), bindings: bindings)
                .buttonMask, 16)
    }

    @MainActor
    func testCaptureSessionAutomaticallySavesAndCancelsOnDeviceChange() throws {
        let suite = "XBXRC.Capture.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        var controller: GCController? = GCController.withExtendedGamepad()
        let input = StreamInputSession(preferences: { settings.mobile }, controller: { controller })
        let pad = try XCTUnwrap(controller?.extendedGamepad)
        let capture = MobileControllerCaptureSession()
        pad.buttonA.setValue(1)
        capture.begin(.shortcut(.diagnostics), input: input)
        XCTAssertTrue(input.isSuppressed)
        capture.poll(input: input, settings: settings)
        pad.buttonA.setValue(0)
        capture.poll(input: input, settings: settings)
        pad.buttonB.setValue(1)
        pad.rightTrigger.setValue(1)
        capture.poll(input: input, settings: settings)
        pad.buttonB.setValue(0)
        capture.poll(input: input, settings: settings)
        XCTAssertNotNil(capture.target)
        pad.rightTrigger.setValue(0)
        capture.poll(input: input, settings: settings)
        XCTAssertNil(capture.target)
        XCTAssertFalse(input.isSuppressed)
        XCTAssertEqual(settings.mobile.diagnosticsButtons, [.b, .rightTrigger])
        XCTAssertEqual(
            AppSettingsStore(defaults: defaults).mobile.diagnosticsButtons, [.b, .rightTrigger])
        capture.begin(.control("a"), input: input)
        input.setSuppressed(true, reason: .lifecycle)
        controller = nil
        capture.poll(input: input, settings: settings)
        XCTAssertNil(capture.target)
        XCTAssertTrue(input.isSuppressed)
        input.setSuppressed(false, reason: .lifecycle)
        XCTAssertFalse(input.isSuppressed)
    }

    func testMappingProfilesRemainSeparateAndRejectInvalidChordTargets() throws {
        let preferences = MobilePreferences()
        var value = preferences
        value.gamepadBindings = [
            "first": ["a": .buttons([.b])], "second": ["a": .buttons([.x])],
            "bad": ["leftX": .buttons([.a]), "x": .buttons([.a, .b]), "y": .buttons([.b, .a])],
        ]
        let normalized = value.normalized()
        XCTAssertEqual(normalized.gamepadBindings["first"]?["a"], .buttons([.b]))
        XCTAssertEqual(normalized.gamepadBindings["second"]?["a"], .buttons([.x]))
        XCTAssertNil(normalized.gamepadBindings["bad"]?["leftX"])
        XCTAssertEqual(normalized.gamepadBindings["bad"]?.count, 1)
        XCTAssertEqual(
            try JSONDecoder().decode(
                MobilePreferences.self, from: JSONEncoder().encode(normalized)), normalized)
    }

    func testLocaleNormalizerProducesStableAchievementCacheLocales() {
        XCTAssertEqual(PreferredGameLocaleNormalizer.normalize("zh_CN"), "zh-CN")
        XCTAssertEqual(PreferredGameLocaleNormalizer.normalize("  "), "en-US")
        XCTAssertEqual(PreferredGameLocaleNormalizer.normalize("EN-us"), "en-US")
    }

    func testCloudCatalogCacheKeyDoesNotExposeAccountOrRegion() {
        let scope = CloudCatalogScope(
            accountID: "sensitive-account-id",
            regionHost: "westus.example.com",
            language: "zh-CN",
            market: "CN"
        )

        XCTAssertEqual(scope.cacheKey.count, 130)
        XCTAssertFalse(scope.cacheKey.contains(scope.accountID))
        XCTAssertFalse(scope.cacheKey.contains(scope.regionHost))
    }

    func testImagePreferenceSaveUpdatesBaseWithoutCreatingOverlay() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("XBXRC-CloudCatalogTests-\(UUID().uuidString)")
        let repository = CloudCatalogSnapshotRepository(rootDirectory: root)
        let scope = CloudCatalogScope(
            accountID: "account",
            regionHost: "region.example.com",
            language: "en-US",
            market: "US"
        )
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: .now,
            overlayUpdatedAt: .now,
            successfulImageURLs: [
                "product": try XCTUnwrap(URL(string: "https://example.com/image.jpg"))
            ]
        )

        try await repository.saveSuccessfulImagePreferences(snapshot)

        let files = try FileManager.default.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isExcludedFromBackupKey]
        )
        XCTAssertEqual(files.count, 1)
        let file = try XCTUnwrap(files.first)
        XCTAssertTrue(file.lastPathComponent.hasPrefix("base-v1-"))
        XCTAssertEqual(
            try file.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup,
            true
        )
    }

    func testImagePreferenceSavePreservesExistingOverlayAndBaseTimestamp() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("XBXRC-CloudCatalogTests-\(UUID().uuidString)")
        let repository = CloudCatalogSnapshotRepository(rootDirectory: root)
        let scope = CloudCatalogScope(
            accountID: "account",
            regionHost: "region.example.com",
            language: "en-US",
            market: "US"
        )
        let baseUpdatedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let overlayUpdatedAt = Date(timeIntervalSince1970: 1_700_000_100)
        let initial = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: baseUpdatedAt,
            overlayUpdatedAt: overlayUpdatedAt
        )
        try await repository.save(initial)

        let imageUpdate = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: .now,
            overlayUpdatedAt: .now,
            successfulImageURLs: [
                "product": try XCTUnwrap(URL(string: "https://example.com/image.jpg"))
            ]
        )
        try await repository.saveSuccessfulImagePreferences(imageUpdate)
        let loaded = try await repository.load(scope: scope)
        let restored = try XCTUnwrap(loaded)

        XCTAssertEqual(restored.baseUpdatedAt, baseUpdatedAt)
        XCTAssertEqual(restored.overlayUpdatedAt, overlayUpdatedAt)
        XCTAssertEqual(restored.successfulImageURLs, imageUpdate.successfulImageURLs)
    }
}

extension SettingsAndPersistenceTests {
    @MainActor
    func testLegacyMobilePreferencesKeepValuesWhenNewSettingsAreAdded() throws {
        let suite = "XBXRC.SettingsParity.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let legacy = Data(
            """
            {"schemaVersion":2,"catalogMarket":"JP","catalogLanguage":"ja-JP","cloudServerRegion":"JAPANEAST","videoFit":"fill","touchOpacity":0.8,"invertRightY":true,"vibrationIntensity":0.4}
            """.utf8)
        defaults.set(legacy, forKey: MobilePreferences.persistenceKey)
        let store = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(store.mobile.catalogMarket, "JP")
        XCTAssertEqual(store.mobile.touchOpacity, 0.8)
        XCTAssertTrue(store.mobile.invertRightY)
        XCTAssertEqual(store.mobile.picture, MobilePicturePreferences())
        XCTAssertEqual(store.mobile.appLanguage, .system)
        store.mobile.picture.apply(.soft)
        store.mobile.menuShortcut = .bumpers
        store.mobile.appLanguage = .english
        let restored = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(restored.mobile.picture.colorPreset, .soft)
        XCTAssertEqual(restored.mobile.menuShortcut, .bumpers)
        XCTAssertEqual(restored.mobile.appLanguage, .english)
        XCTAssertEqual(restored.preferredGameLocale, "en-US")
        XCTAssertEqual(restored.streamingSessionSettings.cloudServerRegion, "JAPANEAST")
    }

    func testPictureNormalizationKeepsFiniteBoundedValuesAndIndependentEffects() {
        var mobile = MobilePreferences()
        mobile.picture.saturation = .infinity
        mobile.picture.contrast = -1
        mobile.picture.brightness = 900
        mobile.picture.sharpness = 2
        mobile.picture.superResolution = true
        mobile.picture.compatibilityMode = true
        mobile.videoFit = "ratio4x3"
        let normalized = mobile.normalized()
        XCTAssertEqual(normalized.picture.saturation, 100)
        XCTAssertEqual(normalized.picture.contrast, 0)
        XCTAssertEqual(normalized.picture.brightness, 200)
        XCTAssertEqual(normalized.picture.sharpness, 1)
        XCTAssertTrue(normalized.picture.superResolution)
        XCTAssertTrue(normalized.picture.compatibilityMode)
        XCTAssertEqual(normalized.videoFit, "ratio4x3")
        mobile.videoFit = "invalid"
        XCTAssertEqual(mobile.normalized().videoFit, "fit")
    }

    func testTurnImportValidatesEntireConfigurationAndSize() throws {
        func data(_ uri: String, username: String = " user ") throws -> Data {
            try JSONSerialization.data(withJSONObject: [
                "server_url": uri, "server_username": username, "server_credential": " secret ",
            ])
        }
        let valid = try MobileTurnConfiguration.imported(
            from: data("turns:relay.example.com:5349?transport=tcp"))
        XCTAssertEqual(valid.username, "user")
        XCTAssertEqual(valid.credential, "secret")
        for uri in [
            "https:relay.example.com", "turn:", "turn://relay.example.com",
            "turn:user@relay.example.com",
            "turn:relay.example.com:0", "turn:relay.example.com:99999",
            "turn:relay.example.com/path",
            "turn:relay.example.com?transport=bad", "turn:relay.example.com#fragment",
        ] {
            XCTAssertThrowsError(try MobileTurnConfiguration.imported(from: data(uri)), uri)
        }
        XCTAssertThrowsError(
            try MobileTurnConfiguration.imported(from: data("turn:relay.example.com", username: ""))
        )
        XCTAssertThrowsError(
            try MobileTurnConfiguration.imported(from: Data(repeating: 0, count: 16_385)))
    }

    @MainActor
    func testSelectedControllerChordPausesInputAndOpensOnceAfterHold() throws {
        let controller = GCController.withExtendedGamepad()
        let gamepad = try XCTUnwrap(controller.extendedGamepad)
        var preferences = MobilePreferences()
        preferences.menuShortcut = .bumpers
        let input = StreamInputSession(preferences: { preferences }, controller: { controller })
        XCTAssertEqual(input.outputState(now: 0), .neutral)
        gamepad.buttonMenu.setValue(1)
        gamepad.buttonOptions?.setValue(1)
        _ = input.outputState(now: 100)
        _ = input.outputState(now: 800)
        XCTAssertEqual(input.menuRevealToken, 0)
        gamepad.buttonMenu.setValue(0)
        gamepad.buttonOptions?.setValue(0)
        gamepad.leftShoulder.setValue(1)
        gamepad.rightShoulder.setValue(1)
        XCTAssertEqual(input.outputState(now: 900), .neutral)
        _ = input.outputState(now: 1_600)
        _ = input.outputState(now: 2_600)
        XCTAssertEqual(input.menuRevealToken, 1)
        gamepad.leftShoulder.setValue(0)
        gamepad.rightShoulder.setValue(0)
        _ = input.outputState(now: 2_700)
        gamepad.leftShoulder.setValue(1)
        gamepad.rightShoulder.setValue(1)
        _ = input.outputState(now: 2_800)
        _ = input.outputState(now: 3_500)
        XCTAssertEqual(input.menuRevealToken, 2)
    }

    func testShortcutMigrationNormalizationAndConflictPreserveMenu() throws {
        let legacy = try JSONDecoder().decode(
            MobilePreferences.self, from: Data(#"{"menuShortcut":"stickButtons"}"#.utf8))
        XCTAssertEqual(legacy.shortcutButtons(for: .menu), [.leftStick, .rightStick])
        XCTAssertTrue(legacy.diagnosticsButtons.isEmpty)
        var preferences = legacy
        preferences.customMenuButtons = [.rightTrigger, .a, .a, .leftTrigger]
        preferences.diagnosticsButtons = [.leftTrigger, .rightTrigger, .a]
        preferences = preferences.normalized()
        XCTAssertEqual(preferences.customMenuButtons, [.a, .leftTrigger, .rightTrigger])
        XCTAssertTrue(preferences.diagnosticsButtons.isEmpty)
        XCTAssertTrue(
            preferences.shortcutConflicts([.rightTrigger, .a, .leftTrigger], for: .diagnostics))
        preferences.setShortcut([], for: .menu)
        let restored = try JSONDecoder().decode(
            MobilePreferences.self, from: JSONEncoder().encode(preferences))
        XCTAssertEqual(restored.customMenuButtons, [])
        XCTAssertTrue(restored.shortcutButtons(for: .menu).isEmpty)
        XCTAssertEqual(MobileShortcutButton.normalized(MobileShortcutButton.allCases).count, 4)
    }

    func testVibrationPresetsMatchPCValuesBeforeHapticsRoutingAndPreserveLegacyGain() throws {
        let legacy = try JSONDecoder().decode(
            MobilePreferences.self, from: Data(#"{"vibrationIntensity":0.4}"#.utf8))
        XCTAssertEqual(legacy.vibrationChannels.uniformStrength, .full)
        XCTAssertEqual(legacy.vibrationIntensity, 0.4)
        XCTAssertEqual(MobilePreferences().vibrationChannels.uniformStrength, .realistic)
        XCTAssertEqual(
            MobileVibrationStrength.realistic.magnitude(1, trigger: false), 0.66, accuracy: 0.0001)
        XCTAssertEqual(
            MobileVibrationStrength.enhanced.magnitude(1, trigger: true), 0.74, accuracy: 0.0001)
        XCTAssertEqual(MobileVibrationStrength.realistic.magnitude(.nan, trigger: true), 0)
        var preferences = MobilePreferences()
        preferences.vibrationChannels.leftHandle = .full
        preferences.vibrationChannels.leftTrigger = .enhanced
        let raw = IOSStreamRumbleEffect(
            gamepadIndex: 0, strongMagnitude: 1, weakMagnitude: 1, leftTrigger: 1,
            rightTrigger: 1, durationMilliseconds: 200, format: .betterXcloud,
            delayMilliseconds: 40, repeatCount: 2)
        let effect = preferences.vibrationEffect(raw)
        let pulses = IOSStreamHapticsRouter.pulses(
            for: effect, supportedRoutes: [.leftHandle, .rightHandle, .leftTrigger, .rightTrigger])
        XCTAssertEqual(pulses.count, 4)
        XCTAssertEqual(
            try XCTUnwrap(pulses.first { $0.route == .leftHandle }).intensity, 1, accuracy: 0.0001)
        XCTAssertEqual(
            try XCTUnwrap(pulses.first { $0.route == .rightHandle }).intensity, 0.66,
            accuracy: 0.0001)
        XCTAssertEqual(
            try XCTUnwrap(pulses.first { $0.route == .leftTrigger }).intensity, 0.74,
            accuracy: 0.0001)
        XCTAssertEqual(
            try XCTUnwrap(pulses.first { $0.route == .rightTrigger }).intensity, 0.58,
            accuracy: 0.0001)
        XCTAssertEqual(effect.durationMilliseconds, raw.durationMilliseconds)
        XCTAssertEqual(effect.delayMilliseconds, raw.delayMilliseconds)
        XCTAssertEqual(effect.repeatCount, raw.repeatCount)
        XCTAssertEqual(
            try JSONDecoder().decode(
                MobilePreferences.self, from: JSONEncoder().encode(preferences)), preferences)
    }

    func testShortcutMatcherUsesActualTriggerThresholdAndRearmsAfterFullRelease() {
        var preferences = MobilePreferences()
        preferences.setShortcut([.leftShoulder], for: .menu)
        preferences.setShortcut([.leftShoulder, .leftTrigger], for: .diagnostics)
        var matcher = MobileShortcutMatcher()
        var state = IOSStreamGamepadState.neutral
        state.buttonMask = 4096
        state.leftTrigger = 0.51
        XCTAssertTrue(matcher.sample(state, preferences: preferences, now: 0).captured)
        XCTAssertEqual(
            matcher.sample(state, preferences: preferences, now: 700).action, .diagnostics)
        XCTAssertNil(matcher.sample(state, preferences: preferences, now: 1_500).action)
        state.leftTrigger = 0
        XCTAssertTrue(matcher.sample(state, preferences: preferences, now: 1_600).captured)
        XCTAssertNil(matcher.sample(state, preferences: preferences, now: 2_500).action)
        state = .neutral
        XCTAssertFalse(matcher.sample(state, preferences: preferences, now: 2_600).captured)
        state.buttonMask = 4096
        state.leftTrigger = 0.5
        _ = matcher.sample(state, preferences: preferences, now: 2_700)
        XCTAssertEqual(matcher.sample(state, preferences: preferences, now: 3_400).action, .menu)
    }

    @MainActor
    func testDiagnosticsShortcutSuppressesLifecycleAndWaitsForNeutralAfterRebinding() throws {
        let controller = GCController.withExtendedGamepad()
        let gamepad = try XCTUnwrap(controller.extendedGamepad)
        let suiteName = "XBXRC.ShortcutTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let settings = AppSettingsStore(defaults: defaults)
        var preferences = MobilePreferences()
        preferences.setShortcut([.a, .rightTrigger], for: .diagnostics)
        settings.mobile = preferences
        let input = StreamInputSession(preferences: { settings.mobile }, controller: { controller })
        gamepad.buttonA.setValue(1)
        gamepad.rightTrigger.setValue(1)
        _ = input.outputState(now: 0)
        _ = input.outputState(now: 1_000)
        XCTAssertEqual(input.diagnosticsRevealToken, 0, "连接时按住组合应等待归零")
        gamepad.buttonA.setValue(0)
        gamepad.rightTrigger.setValue(0)
        _ = input.outputState(now: 1_100)
        gamepad.buttonA.setValue(1)
        gamepad.rightTrigger.setValue(1)
        _ = input.outputState(now: 1_200)
        XCTAssertEqual(input.outputState(now: 1_900), .neutral)
        XCTAssertEqual(input.diagnosticsRevealToken, 1)
        input.setSuppressed(true, reason: .lifecycle)
        _ = input.outputState(now: 3_000)
        XCTAssertEqual(input.diagnosticsRevealToken, 1)
        input.setSuppressed(false, reason: .lifecycle)
        _ = input.outputState(now: 4_000)
        XCTAssertEqual(input.diagnosticsRevealToken, 1)
        preferences.setShortcut([.a], for: .diagnostics)
        settings.mobile = preferences
        XCTAssertEqual(AppSettingsStore(defaults: defaults).mobile.diagnosticsButtons, [.a])
        _ = input.outputState(now: 4_100)
        _ = input.outputState(now: 5_000)
        XCTAssertEqual(input.diagnosticsRevealToken, 1)
        gamepad.buttonA.setValue(0)
        gamepad.rightTrigger.setValue(0)
        _ = input.outputState(now: 5_100)
        gamepad.buttonA.setValue(1)
        _ = input.outputState(now: 5_200)
        _ = input.outputState(now: 5_900)
        XCTAssertEqual(input.diagnosticsRevealToken, 2)
        input.reset()
        _ = input.outputState(now: 7_000)
        XCTAssertEqual(input.diagnosticsRevealToken, 2)
    }

    func testDiagnosticCaptureWorksWithBackgroundOffAndKeepsRedaction() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .off)
        try await writer.startCapture()
        XCTAssertTrue(writer.isCapturing)
        let active = await writer.diagnosticRecords()
        XCTAssertEqual(active.count, 1)
        XCTAssertTrue(active[0].isActive)
        XCTAssertEqual(writer.currentProfile, .off)
        writer.record(
            IOSRuntimeTraceDraft(
                category: .event, domain: "settings", event: "testCapture",
                payload: [
                    "refreshToken": .string("private-token"),
                    "serverCredential": .string("private-secret"),
                ],
                dimension: .core, importance: .essential, operationID: nil))
        _ = await writer.stopCapture()
        XCTAssertFalse(writer.isCapturing)
        let records = await writer.diagnosticRecords()
        XCTAssertFalse(records[0].isActive)
        XCTAssertNotNil(records[0].endedAt)
        let export = try await writer.diagnosticExport(records[0])
        defer { try? FileManager.default.removeItem(at: export) }
        let content = try String(contentsOf: export, encoding: .utf8)
        XCTAssertTrue(content.contains("testCapture"))
        XCTAssertFalse(content.contains("private-token"))
        XCTAssertFalse(content.contains("private-secret"))
        let repeatedStop = await writer.stopCapture()
        XCTAssertNil(repeatedStop)
        await writer.clearFiles()
        let cleared = await writer.diagnosticRecords()
        XCTAssertTrue(cleared.isEmpty)
    }

    func testDiagnosticCaptureRetentionKeepsThreeRecords() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .production)
        for _ in 0..<5 {
            try await writer.startCapture()
            _ = await writer.stopCapture()
        }
        let records = await writer.diagnosticRecords()
        XCTAssertEqual(records.count, 3)
    }

    func testTraceWriterCanReleaseLastReferenceOnItsIOQueue() async {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let queue = DispatchQueue(label: "XBXRC.TraceWriterDeinitTest")
        queue.suspend()
        var writer: IOSRuntimeTraceWriter? = IOSRuntimeTraceWriter(
            rootDirectory: root, profile: .off, queue: queue)
        weak var releasedWriter = writer
        queue.async { [retainedWriter = writer!] in
            withExtendedLifetime(retainedWriter) {}
        }
        writer = nil
        queue.resume()
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume() }
        }
        XCTAssertNil(releasedWriter, "写入队列上的最后一份引用应能完成析构")
    }

    func testFailureDiagnosticsSaveRedactedSnapshotWithoutManualCapture() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .production)
        writer.record(
            IOSRuntimeTraceDraft(
                category: .event, domain: "ios-auth", event: "authSignInFailed",
                payload: ["refreshToken": .string("secret-value")], dimension: .core,
                importance: .key, operationID: nil))
        await writer.flush()
        let records = await writer.diagnosticRecords()
        XCTAssertEqual(records.count, 1)
        XCTAssertEqual(records[0].operation, "ios-auth")
        let export = try await writer.diagnosticExport(records[0])
        defer { try? FileManager.default.removeItem(at: export) }
        let text = try String(contentsOf: export, encoding: .utf8)
        XCTAssertTrue(text.contains("authSignInFailed"))
        XCTAssertFalse(text.contains("secret-value"))
    }

    func testLightweightDiagnosticsRetainsFailuresAndManualCaptureDetail() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .dev)
        let previous = UserDefaults.standard.bool(forKey: "ios.diagnostics.lightweight")
        defer { writer.setLightweight(previous) }
        writer.setLightweight(true)
        let draft = IOSRuntimeTraceDraft(
            category: .snapshot, domain: "settings", event: "detailSnapshot",
            payload: [:], dimension: .core, importance: .debug, operationID: nil)
        writer.record(draft)
        await writer.flush()
        let files = await writer.traceFiles()
        let before = try String(contentsOf: XCTUnwrap(files.last), encoding: .utf8)
        XCTAssertFalse(before.contains("detailSnapshot"))
        try await writer.startCapture()
        writer.record(draft)
        _ = await writer.stopCapture()
        let records = await writer.diagnosticRecords()
        let export = try await writer.diagnosticExport(XCTUnwrap(records.first))
        defer { try? FileManager.default.removeItem(at: export) }
        let captured = try String(contentsOf: export, encoding: .utf8)
        XCTAssertTrue(captured.contains("detailSnapshot"))
    }

    @MainActor
    func testMetalPictureShadersCompileOnAvailableDevice() throws {
        guard let device = MTLCreateSystemDefaultDevice() else {
            throw XCTSkip("Metal device unavailable")
        }
        XCTAssertTrue(StreamingMetalVideoSurfaceView.supportsPictureRendering(on: device))
    }
}

final class TraceReliabilityTests: XCTestCase, @unchecked Sendable {
    private func root() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(
            "trace-reliability-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func draft(
        _ event: String, importance: IOSRuntimeTraceImportance = .key,
        category: IOSRuntimeTraceCategory = .state, payload: [String: IOSRuntimeTraceValue] = [:]
    ) -> IOSRuntimeTraceDraft {
        IOSRuntimeTraceDraft(
            category: category, domain: "ios-streaming", event: event,
            payload: payload, dimension: .core, importance: importance, operationID: "trace-test")
    }

    private func rows(_ writer: IOSRuntimeTraceWriter) async throws -> [IOSRuntimeTraceEnvelope] {
        var rows: [IOSRuntimeTraceEnvelope] = []
        for file in await writer.traceFiles() {
            let content = try String(contentsOf: file, encoding: .utf8)
            for line in content.split(separator: "\n") {
                rows.append(
                    try JSONDecoder().decode(IOSRuntimeTraceEnvelope.self, from: Data(line.utf8)))
            }
        }
        return rows
    }

    func testUnavailableStorageHasBoundedMemoryExplicitExportErrorAndRecovery() async throws {
        let parent = try root()
        let blocked = parent.appendingPathComponent("blocked")
        try Data("ordinary file".utf8).write(to: blocked)
        let writer = IOSRuntimeTraceWriter(rootDirectory: blocked, profile: .dev)
        for _ in 0..<10 { writer.record(draft("lostEvent")) }
        await writer.flush()
        let failed = await writer.status()
        XCTAssertNotNil(failed.storageFailure)
        XCTAssertEqual(failed.bufferedBytes, 0)
        XCTAssertEqual(failed.storageDroppedRows, 10)
        let activeURL = await writer.currentFileURL()
        XCTAssertNil(activeURL)
        do {
            _ = try await writer.prepareExport(allFiles: true)
            XCTFail("存储故障应返回明确错误")
        } catch { XCTAssertTrue(error is IOSRuntimeTraceStorageFailure) }
        try FileManager.default.removeItem(at: blocked)
        try FileManager.default.createDirectory(at: blocked, withIntermediateDirectories: true)
        await writer.flush()
        writer.record(draft("recoveredEvent"))
        let recovered = try await rows(writer)
        XCTAssertTrue(recovered.contains { $0.event == "traceStorageRecovered" })
        XCTAssertTrue(recovered.contains { $0.event == "recoveredEvent" })
        XCTAssertTrue(recovered.contains { $0.payload["storageDroppedRows"] == .integer(10) })
    }

    func testWriteFailureCountsLostTailAndRecoversWithoutRepeatingPrefix() async throws {
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev)
        writer.record(draft("persistedPrefix"))
        await writer.flush()
        let handle = try XCTUnwrap(
            Mirror(reflecting: writer).children.first { $0.label == "fileHandle" }?.value
                as? FileHandle)
        try handle.close()
        for _ in 0..<4 { writer.record(draft("lostTail")) }
        await writer.flush()
        let failed = await writer.status()
        XCTAssertEqual(failed.storageFailure?.stage, "write")
        XCTAssertEqual(failed.storageDroppedRows, 4)
        XCTAssertEqual(failed.bufferedBytes, 0)
        await writer.flush()
        writer.record(draft("afterRecovery"))
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.event == "persistedPrefix" }.count, 1)
        XCTAssertEqual(exported.filter { $0.event == "afterRecovery" }.count, 1)
        XCTAssertEqual(exported.filter { $0.event == "lostTail" }.count, 0)
        XCTAssertEqual(Set(exported.map(\.seq)).count, exported.count)
    }

    func testQueueHasHardCapacityReservedAnchorsAndExactDropSummary() async throws {
        let q = DispatchQueue(label: "trace.pressure.regression")
        q.suspend()
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        for _ in 0..<5000 { writer.record(draft("keyEvent")) }
        for _ in 0..<50 { writer.record(draft("debugEvent", importance: .debug)) }
        for _ in 0..<200 { writer.record(draft("essentialEvent", importance: .essential)) }
        q.resume()
        await writer.flush()
        let status = await writer.status()
        XCTAssertEqual(status.highWaterRows, IOSRuntimeTracePolicy.pendingRowLimit)
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.event == "keyEvent" }.count, 3968)
        XCTAssertEqual(exported.filter { $0.event == "essentialEvent" }.count, 128)
        let notices = exported.filter { $0.event == "traceBudgetNotice" }
        func sum(_ field: String) -> Int64 {
            notices.reduce(0) { sum, row in
                if case .integer(let n) = row.payload[field] { sum + n } else { sum }
            }
        }
        XCTAssertEqual(sum("keyDropped"), 1032)
        XCTAssertEqual(sum("essentialDropped"), 72)
        XCTAssertEqual(sum("debugDropped"), 50)
    }

    func testFlushIncludesRemainingDropsWithinNoticeInterval() async throws {
        let q = DispatchQueue(label: "trace.tail.regression")
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        for _ in 0..<2 {
            q.suspend()
            for _ in 0..<2100 { writer.record(draft("debugEvent", importance: .debug)) }
            q.resume()
            await writer.flush()
        }
        let notices = try await rows(writer).filter { $0.event == "traceBudgetNotice" }
        let lost = notices.reduce(Int64(0)) { sum, row in
            if case .integer(let n) = row.payload["debugDropped"] { sum + n } else { sum }
        }
        XCTAssertEqual(lost, 104)
        XCTAssertTrue(notices.contains { $0.payload["finalSummary"] == .bool(true) })
    }

    func testProfileDisableFlushesAcceptedEvidenceAndDropSummary() async throws {
        let previous = UserDefaults.standard.object(
            forKey: IOSRuntimeTracePolicy.profileDefaultsKey)
        defer {
            UserDefaults.standard.set(previous, forKey: IOSRuntimeTracePolicy.profileDefaultsKey)
        }
        let q = DispatchQueue(label: "trace.profile.regression")
        q.suspend()
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        for _ in 0..<2100 { writer.record(draft("debugEvent", importance: .debug)) }
        writer.setProfile(.off)
        q.resume()
        await writer.flush()
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.event == "debugEvent" }.count, 2048)
        XCTAssertTrue(exported.contains { $0.payload["debugDropped"] == .integer(52) })
        XCTAssertEqual(writer.currentProfile, .off)
    }

    func testTimestampUsesEventSourceWhileQueueDelayIsSeparate() async throws {
        let q = DispatchQueue(label: "trace.timestamp.regression")
        q.suspend()
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        let event = draft("delayedEvent")
        writer.record(event)
        try await Task.sleep(for: .milliseconds(60))
        q.resume()
        let exported = try await rows(writer)
        let row = try XCTUnwrap(exported.first { $0.event == "delayedEvent" })
        XCTAssertEqual(row.tsMs, event.tsMs)
        if case .double(let delay) = row.payload["writerQueueDelayMs"] {
            XCTAssertGreaterThanOrEqual(delay, 50)
        } else {
            XCTFail("应有独立队列等待时间")
        }
    }

    func testOversizedRowIsCountedAndSubsequentEvidencePersists() async throws {
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev)
        writer.record(
            draft("oversized", payload: ["text": .string(String(repeating: "x", count: 100_000))]))
        writer.record(draft("afterOversized"))
        let exported = try await rows(writer)
        XCTAssertFalse(exported.contains { $0.event == "oversized" })
        XCTAssertTrue(exported.contains { $0.event == "afterOversized" })
        let status = await writer.status()
        XCTAssertEqual(status.storageDroppedRows, 1)
        XCTAssertTrue(exported.contains { $0.payload["storageDropped"] == .integer(1) })
    }

    func testLightweightKeepsHealthAndSamplesPerFrameEvidence() async throws {
        let previous = UserDefaults.standard.bool(forKey: "ios.diagnostics.lightweight")
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev)
        writer.setLightweight(true)
        defer { writer.setLightweight(previous) }
        for event in ["rtcHealthSnapshot", "displayHealthSnapshot", "runtimeSupervisionSnapshot"] {
            writer.record(draft(event, category: .snapshot))
        }
        for count in 1...120 {
            writer.record(
                draft("displayPresent", payload: ["presentCount": .integer(Int64(count))]))
        }
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.category == "snapshot" }.count, 3)
        XCTAssertEqual(exported.filter { $0.event == "displayPresent" }.count, 3)
        try await writer.startCapture()
        writer.record(draft("displayPresent", payload: ["presentCount": .integer(121)]))
        let captureURL = await writer.stopCapture()
        let url = try XCTUnwrap(captureURL)
        XCTAssertTrue(try String(contentsOf: url, encoding: .utf8).contains("displayPresent"))
    }

    func testPacketStatisticsUseVideoScopeAndPreserveMissingObservations() {
        var packets = StreamingVideoPacketStatistics()
        packets.observe(kind: "video", received: 100, lost: 2)
        packets.observe(kind: "audio", received: 500, lost: 80)
        packets.observe(kind: "video", received: 50, lost: 1)
        XCTAssertEqual(packets.received, 150)
        XCTAssertEqual(packets.lost, 3)
        XCTAssertEqual(packets.observedLoss, 3)
        packets.observe(kind: "video", received: 10, lost: nil)
        XCTAssertNil(packets.observedLoss)
    }

    func testCounterWindowsPreserveZeroSupplyAndResetOnGenerationOrRegression() {
        var window = StreamingTraceCounterWindow()
        XCTAssertEqual(
            window.observe(at: 1000, scope: "1", counters: ["presentCount": 2])[
                "presentCountDelta"],
            .null)
        let growing = window.observe(at: 2000, scope: "1", counters: ["presentCount": 62])
        XCTAssertEqual(growing["presentCountDelta"], .integer(60))
        XCTAssertEqual(growing["sampleWindowMs"], .double(1000))
        XCTAssertEqual(
            window.observe(at: 3000, scope: "1", counters: ["presentCount": 62])[
                "presentCountDelta"],
            .integer(0))
        let reset = window.observe(at: 4000, scope: "1", counters: ["presentCount": 1])
        XCTAssertEqual(reset["presentCountDelta"], .null)
        XCTAssertEqual(reset["counterReset"], .bool(true))
        XCTAssertEqual(
            window.observe(at: 5000, scope: "2", counters: ["presentCount": 60])[
                "presentCountDelta"],
            .null)
    }

    func testGpuAndDrawableLatencyHaveIndependentSourcesAndBoundedWindows() {
        let telemetry = StreamingDisplayTelemetry()
        for index in 1...300 {
            let submittedAt = Double(index) * 16
            _ = telemetry.recordSubmit(
                identifier: UInt64(index), timestampMilliseconds: submittedAt)
            _ = telemetry.recordPresent(
                identifier: UInt64(index), timestampMilliseconds: submittedAt + 2)
            _ = telemetry.recordDrawablePresent(
                submittedAt: submittedAt, presentedAt: submittedAt + 22)
        }
        let snapshot = telemetry.setDrawableBusy(false)
        XCTAssertEqual(snapshot.displayedFrameDelta, 1)
        XCTAssertEqual(snapshot.latencySampleCount, 240)
        XCTAssertEqual(snapshot.drawableSampleCount, 240)
        XCTAssertEqual(snapshot.submitToPresentP95Milliseconds, 2)
        XCTAssertEqual(snapshot.submitToDrawableP95Ms, 22)
        XCTAssertEqual(
            snapshot.tracePayload()["presentTimestampSource"], .string("commandBufferCompletion"))
        XCTAssertEqual(
            snapshot.tracePayload()["drawableTimestampSource"], .string("drawablePresentedTime"))
        XCTAssertEqual(snapshot.drawablePresentCount, 300)
    }
}
