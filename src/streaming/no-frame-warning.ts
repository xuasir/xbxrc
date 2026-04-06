/**
 * 「长时间无画面」浮层与诊断抑制共用。
 * 云游戏/远端画像下首帧往往明显慢于 Home，过短会误报。
 */
export const NO_FRAME_WARNING_DELAY_MS = 60_000

/** 距上次 frameReady 小于此间隔仍视为有输出：定时器到期时顺延而非立刻弹层 */
export const NO_FRAME_RECENT_ACTIVITY_MS = 60_000
