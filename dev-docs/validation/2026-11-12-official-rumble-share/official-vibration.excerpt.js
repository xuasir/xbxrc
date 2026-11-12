class E {
    constructor() { s()(this, "gamepadPlayVibrationStates", void 0), s()(this, "enableVibration", !1), s()(this, "unloadHandler", void 0), this.gamepadPlayVibrationStates = new Map, this.enableVibration = !1, this.unloadHandler = () => this.stop(), window.addEventListener("beforeunload", this.unloadHandler); }
    setEnableVibration(e) { this.enableVibration = e, this.enableVibration || (this.clearAllVibrationTimers(), this.stopAllMotors()); }
    onGamepadConnected(e) { if (!this.enableVibration || this.gamepadPlayVibrationStates.has(e))
        return; const t = this.getGamepad(e); let i = !1, n = !1; var r, s, o, d, h; null !== t && void 0 !== t && t.vibrationActuator && (i = !0, n = (null === (r = (s = t.vibrationActuator).canPlay) || void 0 === r ? void 0 : r.call(s, "trigger-rumble")) || (null === (o = t.vibrationActuator.effects) || void 0 === o ? void 0 : o.includes("trigger-rumble")) || (null === (d = (h = t.vibrationActuator).canPlayEffectType) || void 0 === d ? void 0 : d.call(h, "trigger-rumble"))); this.gamepadPlayVibrationStates.set(e, a()(a()({}, C), {}, { gamepad: t, supportsDualRumble: i, supportsTriggerRumble: n })); }
    onGamepadDisconnected(e) { const t = this.gamepadPlayVibrationStates.get(e); t && this.clearVibrationTimers(t), this.gamepadPlayVibrationStates.delete(e); }
    stop() { this.clearAllVibrationTimers(), this.stopAllMotors(), this.gamepadPlayVibrationStates.clear(), window.removeEventListener("beforeunload", this.unloadHandler); }
    onVibration(e, t) { var i; if (!this.enableVibration)
        return; const n = this.gamepadPlayVibrationStates.get(e); if (n && n.supportsDualRumble && null !== (i = n.gamepad) && void 0 !== i && i.connected) {
        if (this.clearVibrationTimers(n), n.leftMotorPercent = t.leftMotorPercent, n.rightMotorPercent = t.rightMotorPercent, n.leftTriggerMotorPercent = t.leftTriggerMotorPercent, n.rightTriggerMotorPercent = t.rightTriggerMotorPercent, n.durationMs = t.durationMs, n.delayMs = t.delayMs, n.repeat = t.repeat, 0 === n.leftMotorPercent && 0 === n.rightMotorPercent && 0 === n.leftTriggerMotorPercent && 0 === n.rightTriggerMotorPercent)
            return this.stopVibration(e), n.durationMs = 0, void (n.repeat = 0);
        0 !== t.delayMs ? n.timeoutId = setTimeout((() => this.onPlayVibration(e)), t.delayMs) : this.onPlayVibration(e);
    } }
    clearAllVibrationTimers() { for (const e of this.gamepadPlayVibrationStates)
        this.clearVibrationTimers(e[1]); }
    clearVibrationTimers(e) { e.timeoutId && (clearTimeout(e.timeoutId), e.timeoutId = void 0), e.intervalId && (clearInterval(e.intervalId), e.intervalId = void 0); }
    stopAllMotors() { for (const e of this.gamepadPlayVibrationStates)
        this.resetVibrationState(e[1]); }
    stopVibration(e) { const t = this.gamepadPlayVibrationStates.get(e); t && (this.clearVibrationTimers(t), this.resetVibrationState(t)); }
    resetVibrationState(e) { var t; "function" === typeof (null === (t = e.gamepad) || void 0 === t || null === (t = t.vibrationActuator) || void 0 === t ? void 0 : t.reset) ? e.gamepad.vibrationActuator.reset() : (e.leftMotorPercent = 0, e.rightMotorPercent = 0, e.leftTriggerMotorPercent = 0, e.rightTriggerMotorPercent = 0, e.durationMs = 0, e.delayMs = 0, e.repeat = 0, this.playVibration(e)); }
    onPlayVibration(e) { const t = this.gamepadPlayVibrationStates.get(e); t && (this.playVibration(t), t.timeoutId = void 0, t.repeat > 0 ? (t.repeat--, t.intervalId || (t.intervalId = setInterval((() => this.onPlayVibration(e)), t.delayMs + t.durationMs))) : t.intervalId && (clearInterval(t.intervalId), t.intervalId = void 0)); }
    playVibration(e) { var t, i, n; if (null === (t = e.gamepad) || void 0 === t || !t.connected)
        return void (e.repeat = 0); const a = { duration: e.durationMs, startDelay: 0, strongMagnitude: e.leftMotorPercent, weakMagnitude: e.rightMotorPercent, leftTrigger: e.leftTriggerMotorPercent, rightTrigger: e.rightTriggerMotorPercent }; if (!e.supportsTriggerRumble) {
        const t = e.rightMotorPercent < .6 ? (.6 - e.rightMotorPercent) / 2 : 0, i = (e.leftTriggerMotorPercent + e.rightTriggerMotorPercent) / 4, n = Math.min(t, i);
        a.weakMagnitude = Math.min(1, e.rightMotorPercent + n), a.leftTrigger = 0, a.rightTrigger = 0;
    } var r, s; (null === (i = e.gamepad.vibrationActuator) || void 0 === i || null === (n = i.playEffect) || void 0 === n || n.call(i, "dual-rumble", a), e.supportsTriggerRumble) && (null === (r = e.gamepad.vibrationActuator) || void 0 === r || null === (s = r.playEffect) || void 0 === s || s.call(r, "trigger-rumble", a)); }
    getGamepad(e) { var t; const i = navigator.getGamepads(); if (!(i.length <= e))
        return null !== (t = i[e]) && void 0 !== t ? t : void 0; }
}