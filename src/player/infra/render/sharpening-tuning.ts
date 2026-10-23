/**
 * 锐化后处理调参唯一来源。
 *
 * GLSL 模板通过插值引用这里的常量，`sharpening-math.ts` 的参考实现（单测）也引用同一份，
 * 避免 shader 与测试各写一套数值导致观感与断言漂移。
 *
 * 背景：原 CAS 段可化简为 `out = center + (center - mean4) * G`，
 * 其中 `G = 4√a / (5.2 - 4√a)`，平坦区 `G → 3.33`、强边缘 `G → 0`。
 * 也就是说噪声被放大 3.3 倍、真实轮廓几乎不锐化——这就是颗粒感 / 油画感的来源。
 */

/** CAS 自适应曲线的形状系数（沿用原实现，只把增益上限显式化）。 */
export const CAS_EDGE_WEIGHT = 5.2

/** 平坦区高频增益上限。原实现为 3.33，压到 2.0 可让暗部噪声降低约 40%。 */
export const CAS_GAIN_MAX = 2

/** 噪声底：低于该 luma 残差视为噪声。1.5/255 约等于 8bit 量化噪声的 1.5 个码值。 */
export const CAS_NOISE_FLOOR = 1.5 / 255

/** 噪声门限软斜坡倍数，配合 smoothstep 避免硬切造成的细节闪断。 */
export const CAS_GATE_RAMP = 2

/** 邻域 ringing 钳制：允许的外扩比例（相对邻域 range）。与 SR 路径保持一致。 */
export const CAS_RING_PAD = 0.38

/** 邻域钳制混合权重，1.0 表示完全钳制（会抹掉锐化收益），0.92 保留少量过冲。 */
export const CAS_RING_BLEND = 0.92

/** 色度参与锐化的比例：1.0 = 纯 luma 锐化（无色斑），0 = 完全逐通道。 */
export const CAS_CHROMA_BLEND = 0.85

/** USM 分支的增益换算：strength 1.0 等价于旧实现 `sharpenFactor = 4`。 */
export const USM_GAIN_SCALE = 4 / 3
