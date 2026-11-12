// 来源：https://assets.play.xbox.com/playxbox/static/js/8128.1aca8038.chunk.js
// SHA-256：b8f69c5f1c14e589eb665b84ad36dcff54f25e82cbac2640998a8766b32e50d2
// 2026-11-12 源码摘录，别名沿用官方 bundle。

(function (e) { e[e.DpadUp = 0] = "DpadUp", e[e.DpadDown = 1] = "DpadDown", e[e.DpadLeft = 2] = "DpadLeft", e[e.DpadRight = 3] = "DpadRight", e[e.Menu = 4] = "Menu", e[e.View = 5] = "View", e[e.LeftThumb = 6] = "LeftThumb", e[e.RightThumb = 7] = "RightThumb", e[e.LeftShoulder = 8] = "LeftShoulder", e[e.RightShoulder = 9] = "RightShoulder", e[e.Nexus = 10] = "Nexus", e[e.Share = 11] = "Share", e[e.A = 12] = "A", e[e.B = 13] = "B", e[e.X = 14] = "X", e[e.Y = 15] = "Y", e[e.EnumLength = 16] = "EnumLength"; }(y || (y = {})));

// 来源：https://assets.play.xbox.com/playxbox/static/js/web-rtc-stream.d6f6b903.chunk.js
// SHA-256：0e1d97750113681c7c2d1edfc7910c1b2915055450a7cfde8fc7a0a54fef73d2
// 2026-11-12 源码摘录，别名沿用官方 bundle。

(function (e) { e[e.Nexus = 2] = "Nexus", e[e.Menu = 4] = "Menu", e[e.View = 8] = "View", e[e.A = 16] = "A", e[e.B = 32] = "B", e[e.X = 64] = "X", e[e.Y = 128] = "Y", e[e.DpadUp = 256] = "DpadUp", e[e.DpadDown = 512] = "DpadDown", e[e.DpadLeft = 1024] = "DpadLeft", e[e.DpadRight = 2048] = "DpadRight", e[e.LeftShoulder = 4096] = "LeftShoulder", e[e.RightShoulder = 8192] = "RightShoulder", e[e.LeftThumbstick = 16384] = "LeftThumbstick", e[e.RightThumbstick = 32768] = "RightThumbstick"; }(z || (z = {})));


// generateGamepadsFrame
class generateGamepadsFrameExcerpt {
generateGamepadsFrame(e, t, i, n) { const a = n; n = this.appendUint8(this.reportArrayView, n, this.dirtyGamepadMappingCount); for (const r of t) {
    if (!r.Dirty)
        continue;
    n = this.appendUint8(this.reportArrayView, n, r.GamepadIndex);
    let t = 0;
    r.Nexus > 0 && (t |= z.Nexus), r.Menu > 0 && (t |= z.Menu), r.View > 0 && (t |= z.View), r.A > 0 && (t |= z.A), r.B > 0 && (t |= z.B), r.X > 0 && (t |= z.X), r.Y > 0 && (t |= z.Y), r.DPadUp > 0 && (t |= z.DpadUp), r.DPadDown > 0 && (t |= z.DpadDown), r.DPadLeft > 0 && (t |= z.DpadLeft), r.DPadRight > 0 && (t |= z.DpadRight), r.LeftShoulder > 0 && (t |= z.LeftShoulder), r.RightShoulder > 0 && (t |= z.RightShoulder), r.LeftThumb > 0 && (t |= z.LeftThumbstick), r.RightThumb > 0 && (t |= z.RightThumbstick), n = this.appendUint16(this.reportArrayView, n, t), n = this.appendInt16(this.reportArrayView, n, this.normalizeAxisValue(r.LeftThumbXAxis)), n = this.appendInt16(this.reportArrayView, n, this.normalizeAxisValue(r.LeftThumbYAxis)), n = this.appendInt16(this.reportArrayView, n, this.normalizeAxisValue(r.RightThumbXAxis)), n = this.appendInt16(this.reportArrayView, n, this.normalizeAxisValue(r.RightThumbYAxis)), n = this.appendUint16(this.reportArrayView, n, this.normalizeTriggerValue(r.LeftTrigger)), n = this.appendUint16(this.reportArrayView, n, this.normalizeTriggerValue(r.RightTrigger)), e >= 2 && (n = this.appendUint32(this.reportArrayView, n, r.PhysicalPhysicality), n = this.appendUint32(this.reportArrayView, n, r.VirtualPhysicality));
} i !== n - a && u.r.Instance.throw(`InputReporter made a gamepad frame that wasn't the expected size. expected ${i}, actual ${n - a}`, b.ws.BadLength); }
}


// generateUnreliableGamepadFrame
class generateUnreliableGamepadFrameExcerpt {
generateUnreliableGamepadFrame(e, t) { const i = t; t = this.appendUint8(this.reportArrayView, t, e.gamepadId), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.DpadUp]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.DpadDown]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.DpadLeft]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.DpadRight]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.Menu]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.View]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.LeftThumb]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.RightThumb]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.LeftShoulder]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.RightShoulder]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.Nexus]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.Share]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.A]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.B]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.X]), t = this.appendUint8(this.reportArrayView, t, e.transitionCount[c.t.Y]), t = this.appendUint16(this.reportArrayView, t, this.normalizeTriggerValue(e.triggerLeft)), t = this.appendUint16(this.reportArrayView, t, this.normalizeTriggerValue(e.triggerRight)), t = this.appendInt16(this.reportArrayView, t, this.normalizeAxisValue(e.thumbLX)), t = this.appendInt16(this.reportArrayView, t, this.normalizeAxisValue(e.thumbLY)), t = this.appendInt16(this.reportArrayView, t, this.normalizeAxisValue(e.thumbRX)), t = this.appendInt16(this.reportArrayView, t, this.normalizeAxisValue(e.thumbRY)), t = this.appendUint32(this.reportArrayView, t, e.physicalPhysicality), t = this.appendUint32(this.reportArrayView, t, e.virtualPhysicality), this.payloadSizePerUnreliableGamepadFrame !== t - i && u.r.Instance.throw(`InputReporter made a gamepad frame changes that wasn't the expected size. expected ${this.payloadSizePerUnreliableGamepadFrame}, actual ${t - i}`, b.ws.BadLength); }
}


// updateCurrentGamepadInputFrame
class updateCurrentGamepadInputFrameExcerpt {
updateCurrentGamepadInputFrame(e) { for (const t of e) {
    if (!t.Dirty)
        continue;
    const e = this.lastKnownGamepadMappings.findIndex((e => e.GamepadIndex === t.GamepadIndex));
    if (-1 === e) {
        this.logger.error(`InputChannelV2: Gamepad ${t.GamepadIndex} was not added prior to receiving its mapping.`);
        continue;
    }
    const i = this.lastKnownGamepadMappings[e];
    i.A !== t.A && (this.currentFrameState.gamepad.transitionCount[c.t.A]++, i.A = t.A), i.B !== t.B && (this.currentFrameState.gamepad.transitionCount[c.t.B]++, i.B = t.B), i.X !== t.X && (this.currentFrameState.gamepad.transitionCount[c.t.X]++, i.X = t.X), i.Y !== t.Y && (this.currentFrameState.gamepad.transitionCount[c.t.Y]++, i.Y = t.Y), i.LeftShoulder !== t.LeftShoulder && (this.currentFrameState.gamepad.transitionCount[c.t.LeftShoulder]++, i.LeftShoulder = t.LeftShoulder), i.RightShoulder !== t.RightShoulder && (this.currentFrameState.gamepad.transitionCount[c.t.RightShoulder]++, i.RightShoulder = t.RightShoulder), i.View !== t.View && (this.currentFrameState.gamepad.transitionCount[c.t.View]++, i.View = t.View), i.Menu !== t.Menu && (this.currentFrameState.gamepad.transitionCount[c.t.Menu]++, i.Menu = t.Menu), i.LeftThumb !== t.LeftThumb && (this.currentFrameState.gamepad.transitionCount[c.t.LeftThumb]++, i.LeftThumb = t.LeftThumb), i.RightThumb !== t.RightThumb && (this.currentFrameState.gamepad.transitionCount[c.t.RightThumb]++, i.RightThumb = t.RightThumb), i.DPadUp !== t.DPadUp && (this.currentFrameState.gamepad.transitionCount[c.t.DpadUp]++, i.DPadUp = t.DPadUp), i.DPadDown !== t.DPadDown && (this.currentFrameState.gamepad.transitionCount[c.t.DpadDown]++, i.DPadDown = t.DPadDown), i.DPadLeft !== t.DPadLeft && (this.currentFrameState.gamepad.transitionCount[c.t.DpadLeft]++, i.DPadLeft = t.DPadLeft), i.DPadRight !== t.DPadRight && (this.currentFrameState.gamepad.transitionCount[c.t.DpadRight]++, i.DPadRight = t.DPadRight), i.Nexus !== t.Nexus && (this.currentFrameState.gamepad.transitionCount[c.t.Nexus]++, i.Nexus = t.Nexus), this.currentFrameState.gamepad.triggerLeft = t.LeftTrigger, this.currentFrameState.gamepad.triggerRight = t.RightTrigger, this.currentFrameState.gamepad.thumbLX = t.LeftThumbXAxis, this.currentFrameState.gamepad.thumbLY = t.LeftThumbYAxis, this.currentFrameState.gamepad.thumbRX = t.RightThumbXAxis, this.currentFrameState.gamepad.thumbRY = t.RightThumbYAxis, this.currentFrameState.gamepad.physicalPhysicality = t.PhysicalPhysicality, this.currentFrameState.gamepad.virtualPhysicality = t.VirtualPhysicality, this.currentFrameState.gamepad.markStateChange();
} }
}


// setupUnreliableInputModelDataChannels
class setupUnreliableInputModelDataChannelsExcerpt {
setupUnreliableInputModelDataChannels(e, t, i) { const n = { protocol: q.ChannelProtocolV2, ordered: !1, maxRetransmits: 0 }; this.unreliableInputDataChannel = new ce(e.createDataChannel(Q.ChannelName, n)), this.unreliableInputDataChannel.setInternalOpenedCallback((() => { var e; null != (null === (e = this.sdpAnswer) || void 0 === e ? void 0 : e.unreliableinput) ? (this.unreliableInputDataChannel.version = this.sdpAnswer.unreliableinput, this.logger.info(`Unreliable input channel opened with version ${this.unreliableInputDataChannel.version} and config: ${JSON.stringify(n)}`), setTimeout((() => { this.onWebRtcConnectionStateChanged(t, i); }))) : this.logger.info("Server does not support unreliable input channel"); })); const a = { protocol: q.ChannelProtocolV2, ordered: !0, maxRetransmits: void 0, maxPacketLifeTime: void 0 }; this.reliableInputDataChannel = new ce(e.createDataChannel(j.ChannelName, a)), this.reliableInputDataChannel.setInternalOpenedCallback((() => { var e; null != (null === (e = this.sdpAnswer) || void 0 === e ? void 0 : e.reliableinput) ? (this.reliableInputDataChannel.version = this.sdpAnswer.reliableinput, this.logger.info(`Reliable input channel opened with version ${this.reliableInputDataChannel.version} and config: ${JSON.stringify(a)}`), setTimeout((() => { this.onWebRtcConnectionStateChanged(t, i); }))) : this.logger.info("Server does not support reliable input channel (used by unreliable input model)"); })); }
}
