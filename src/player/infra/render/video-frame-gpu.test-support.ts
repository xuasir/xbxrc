import { vi } from 'vitest'

/** 探测器使用独立GL替身，像素在readPixels提交时固定，完成时间可单独控制。 */
export function gpuProbe(video: HTMLVideoElement, spend: (ms: number) => void = () => {}) {
  let image = 20
  let packedImage = 20
  let signaled = true
  let lost = false
  const gl = {
    NO_ERROR: 0,
    FRAMEBUFFER_COMPLETE: 1,
    TIMEOUT_EXPIRED: 2,
    ALREADY_SIGNALED: 3,
    CONDITION_SATISFIED: 4,
    WAIT_FAILED: 5,
    PIXEL_PACK_BUFFER: 6,
    RGBA: 7,
    UNSIGNED_BYTE: 8,
    TEXTURE_2D: 9,
    SYNC_GPU_COMMANDS_COMPLETE: 10,
    createProgram: vi.fn(() => ({})),
    createShader: vi.fn(() => ({})),
    createTexture: vi.fn(() => ({})),
    createFramebuffer: vi.fn(() => ({})),
    createBuffer: vi.fn(() => ({})),
    getShaderParameter: vi.fn(() => true),
    getProgramParameter: vi.fn(() => true),
    checkFramebufferStatus: vi.fn(() => 1),
    getUniformLocation: vi.fn(() => ({})),
    getError: vi.fn(() => 0),
    getExtension: vi.fn(() => ({ loseContext: () => { lost = true } })),
    texImage2D: vi.fn(),
    readPixels: vi.fn(() => { packedImage = image }),
    fenceSync: vi.fn(() => ({})),
    clientWaitSync: vi.fn((_sync: unknown, _flags: number, _timeout: number): number => signaled ? 3 : 2),
    getBufferSubData: vi.fn((_target: number, _offset: number, bytes: Uint8Array) => { bytes.fill(packedImage) }),
    deleteSync: vi.fn(),
    deleteTexture: vi.fn(),
    deleteBuffer: vi.fn(),
    deleteFramebuffer: vi.fn(),
    deleteProgram: vi.fn(),
    deleteShader: vi.fn(),
    shaderSource: vi.fn(),
    compileShader: vi.fn(),
    attachShader: vi.fn(),
    linkProgram: vi.fn(),
    bindTexture: vi.fn(),
    texParameteri: vi.fn(),
    bindFramebuffer: vi.fn(),
    framebufferTexture2D: vi.fn(),
    bindBuffer: vi.fn(),
    bufferData: vi.fn(),
    useProgram: vi.fn(),
    uniform1i: vi.fn(),
    viewport: vi.fn(),
    activeTexture: vi.fn(),
    drawArrays: vi.fn(),
    flush: vi.fn(),
  }
  const listeners = new Map<string, () => void>()
  const canvas = {
    width: 0,
    height: 0,
    getContext: vi.fn(() => gl),
    addEventListener: vi.fn((name: string, fn: () => void) => listeners.set(name, fn)),
    removeEventListener: vi.fn((name: string) => listeners.delete(name)),
  }
  const ownerDocument = { visibilityState: 'visible', createElement: vi.fn(() => canvas) }
  Object.assign(video, { ownerDocument, readyState: 4, paused: false })
  if (!video.videoWidth)
    Object.assign(video, { videoWidth: 2560, videoHeight: 1440 })
  return {
    canvas,
    ownerDocument,
    gl,
    image: (value: number) => { image = value },
    signaled: (value: boolean) => { signaled = value },
    lose: () => listeners.get('webglcontextlost')?.(),
    lost: () => lost,
    spend,
  }
}
