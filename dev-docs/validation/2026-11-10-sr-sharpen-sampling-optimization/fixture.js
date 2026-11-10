function runPixelChecks() {
  const canvas = document.createElement('canvas')
  const gl = canvas.getContext('webgl2', { antialias: false, preserveDrawingBuffer: true })
  if (!gl)
    throw new Error('WebGL2 unavailable')
  const compile = (type, source) => {
    const shader = gl.createShader(type)
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS))
      throw new Error(gl.getShaderInfoLog(shader))
    return shader
  }
  const render = (source, input, width, height, outputWidth = width, outputHeight = height, strength = 0.6, filter = 1) => {
    canvas.width = outputWidth
    canvas.height = outputHeight
    gl.viewport(0, 0, outputWidth, outputHeight)
    const vertex = compile(gl.VERTEX_SHADER, '#version 300 es\nlayout(location=0) in vec4 position;void main(){gl_Position=position;}')
    const fragment = compile(gl.FRAGMENT_SHADER, source)
    const program = gl.createProgram()
    gl.attachShader(program, vertex)
    gl.attachShader(program, fragment)
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(program))
    gl.useProgram(program)
    const buffer = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0)
    const texture = gl.createTexture()
    gl.activeTexture(gl.TEXTURE0)
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, input)
    gl.uniform1i(gl.getUniformLocation(program, 'data'), 0)
    gl.uniform1i(gl.getUniformLocation(program, 'easuTex'), 0)
    gl.uniform1i(gl.getUniformLocation(program, 'filterId'), filter)
    gl.uniform1i(gl.getUniformLocation(program, 'applyColor'), 0)
    gl.uniform1f(gl.getUniformLocation(program, 'sharpenFactor'), strength)
    for (const name of ['brightness', 'contrast', 'saturation'])
      gl.uniform1f(gl.getUniformLocation(program, name), 100)
    gl.uniform2f(gl.getUniformLocation(program, 'sourceResolution'), width, height)
    gl.uniform2f(gl.getUniformLocation(program, 'outputResolution'), outputWidth, outputHeight)
    const constants = computeFsrEasuCon(width, height, width, height, outputWidth, outputHeight)
    for (const name of ['con0', 'con1', 'con2', 'con3'])
      gl.uniform4fv(gl.getUniformLocation(program, name), constants[name])
    gl.drawArrays(gl.TRIANGLES, 0, 3)
    const output = new Uint8Array(outputWidth * outputHeight * 4)
    gl.readPixels(0, 0, outputWidth, outputHeight, gl.RGBA, gl.UNSIGNED_BYTE, output)
    const error = gl.getError()
    gl.deleteTexture(texture)
    gl.deleteBuffer(buffer)
    gl.deleteProgram(program)
    gl.deleteShader(vertex)
    gl.deleteShader(fragment)
    if (error !== gl.NO_ERROR)
      throw new Error(`GPU error: ${error}`)
    return output
  }
  const pattern = (width, height, sample) => {
    const pixels = new Uint8Array(width * height * 4)
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const channels = sample(x, y)
        for (let c = 0; c < 3; c++)
          pixels[(y * width + x) * 4 + c] = channels[c]
        pixels[(y * width + x) * 4 + 3] = 255
      }
    }
    return pixels
  }
  const difference = (a, b) => {
    let max = 0
    let sum = 0
    for (let i = 0; i < a.length; i++) {
      const delta = Math.abs(a[i] - b[i])
      max = Math.max(max, delta)
      sum += delta
    }
    return { maxCodeValue: max, meanCodeValue: sum / a.length }
  }
  const easu = []
  for (const [width, height, ow, oh] of [[7, 5, 7, 5], [8, 6, 12, 9], [7, 5, 14, 10], [13, 9, 18, 13]]) {
    const input = pattern(width, height, (x, y) => [(x * 37 + y * 11) % 256, (x * 3 + y * 53) % 256, (x * 97 + y * 29) % 256])
    const before = render(baselineEasu(true), input, width, height, ow, oh)
    const after = render(buildEasuFragmentSource(true), input, width, height, ow, oh)
    const delta = difference(before, after)
    easu.push({ source: [width, height], output: [ow, oh], ...delta, pass: delta.maxCodeValue <= 1 })
  }
  const usmReference = (input, width, height, strength, outputWidth = width, outputHeight = height) => {
    const output = new Uint8Array(outputWidth * outputHeight * 4)
    const read = (x, y) => {
      const index = (Math.max(0, Math.min(height - 1, y)) * width + Math.max(0, Math.min(width - 1, x))) * 4
      return { r: input[index] / 255, g: input[index + 1] / 255, b: input[index + 2] / 255 }
    }
    const bilinear = (x, y) => {
      const left = Math.floor(x), bottom = Math.floor(y)
      const fx = x - left, fy = y - bottom
      const block = [read(left, bottom), read(left + 1, bottom), read(left, bottom + 1), read(left + 1, bottom + 1)]
      const weights = [(1 - fx) * (1 - fy), fx * (1 - fy), (1 - fx) * fy, fx * fy]
      return Object.fromEntries(['r', 'g', 'b'].map(c => [c, block.reduce((sum, pixel, i) => sum + pixel[c] * weights[i], 0)]))
    }
    const aligned = width === outputWidth && height === outputHeight
    for (let y = 0; y < outputHeight; y++) {
      for (let x = 0; x < outputWidth; x++) {
        const px = (x + 0.5) * width / outputWidth - 0.5
        const py = (y + 0.5) * height / outputHeight - 0.5
        const center = bilinear(px, py)
        const radius = aligned ? 0.5 : 1
        const corners = [[-1, 1], [1, 1], [-1, -1], [1, -1]].map(([sx, sy]) => bilinear(px + sx * radius, py + sy * radius))
        const cross = aligned ? [] : [[0, 1], [-1, 0], [1, 0], [0, -1]].map(([sx, sy]) => bilinear(px + sx, py + sy))
        const filtered = [...corners, ...cross]
        const blur = Object.fromEntries(['r', 'g', 'b'].map(c => [c, aligned
          ? corners.reduce((sum, pixel) => sum + pixel[c], 0) / 4
          : (corners.reduce((sum, pixel) => sum + pixel[c], 0) + 2 * cross.reduce((sum, pixel) => sum + pixel[c], 0) + 4 * center[c]) / 16]))
        const neighborhood = {
          min: Object.fromEntries(['r', 'g', 'b'].map(c => [c, Math.min(center[c], ...filtered.map(pixel => pixel[c]))])),
          max: Object.fromEntries(['r', 'g', 'b'].map(c => [c, Math.max(center[c], ...filtered.map(pixel => pixel[c]))])),
        }
        const result = sharpenUsmPixel(center, blur, strength, neighborhood)
        for (const [channel, index] of ['r', 'g', 'b'].map((c, i) => [c, i]))
          output[(y * outputWidth + x) * 4 + index] = Math.round(result[channel] * 255)
        output[(y * outputWidth + x) * 4 + 3] = 255
      }
    }
    return output
  }
  const patterns = [
    ['flatNoise', (x, y) => [128 + (x + y) % 3 - 1, 128 + (x + y) % 3 - 1, 128 + (x + y) % 3 - 1]],
    ['fineDetail', (x, y) => Array(3).fill((x + y) % 2 ? 131 : 125)],
    ['stepEdge', x => Array(3).fill(x < 4 ? 96 : 160)],
    ['diagonal', (x, y) => Array(3).fill(x > y ? 180 : 70)],
    ['rgbDetail', (x, y) => [(x * 31 + y * 17) % 256, (x * 7 + y * 47) % 256, (x * 71 + y * 13) % 256]],
  ]
  const usm = []
  let noiseEvidence
  let detailEvidence
  let haloEvidence
  let strongHaloEvidence
  for (const [name, sample] of patterns) {
    const input = pattern(9, 7, sample)
    const before = render(baselineSharpen, input, 9, 7)
    const after = render(currentSharpen, input, 9, 7)
    const delta = difference(after, usmReference(input, 9, 7, 0.6))
    usm.push({ name, ...delta, pass: delta.maxCodeValue <= 1 })
    if (name === 'flatNoise')
      noiseEvidence = { baseline: difference(before, input), optimized: difference(after, input), pass: difference(after, input).maxCodeValue <= 1 && difference(after, input).meanCodeValue <= difference(before, input).meanCodeValue }
    if (name === 'fineDetail')
      detailEvidence = { optimized: difference(after, input), pass: difference(after, input).maxCodeValue >= 1 }
    if (name === 'stepEdge') {
      const strongBefore = render(baselineSharpen, input, 9, 7, 9, 7, 1)
      const strongAfter = render(currentSharpen, input, 9, 7, 9, 7, 1)
      strongHaloEvidence = { baseline: difference(strongBefore, input), optimized: difference(strongAfter, input), pass: difference(strongAfter, input).maxCodeValue < difference(strongBefore, input).maxCodeValue }
      haloEvidence = { baseline: difference(before, input), optimized: difference(after, input), pass: difference(after, input).maxCodeValue < difference(before, input).maxCodeValue }
    }
  }
  const colorInput = pattern(9, 7, (x, y) => [(x * 31 + y * 17) % 256, (x * 7 + y * 47) % 256, (x * 71 + y * 13) % 256])
  const scaled = render(currentSharpen, colorInput, 9, 7, 12, 9)
  const scaledDelta = difference(scaled, usmReference(colorInput, 9, 7, 0.6, 12, 9))
  usm.push({ name: 'scaledUSM', ...scaledDelta, pass: scaledDelta.maxCodeValue <= 1 })
  const reconstructed = render(buildEasuFragmentSource(true), colorInput, 9, 7, 12, 9)
  const srUsm = []
  for (const strength of [0, 0.6, 1]) {
    const processed = render(currentSharpen, reconstructed, 12, 9, 12, 9, strength)
    const delta = difference(processed, usmReference(reconstructed, 12, 9, strength))
    srUsm.push({ strength, ...delta, pass: delta.maxCodeValue <= 1 })
  }
  const bypass = difference(render(currentSharpen, colorInput, 9, 7, 9, 7, 0), colorInput)
  const casUnchanged = difference(render(currentSharpen, colorInput, 9, 7, 9, 7, 0.6, 2), render(baselineSharpen, colorInput, 9, 7, 9, 7, 0.6, 2))
  const gates = [noiseEvidence.pass, detailEvidence.pass, haloEvidence.pass, strongHaloEvidence.pass, bypass.maxCodeValue === 0, casUnchanged.maxCodeValue === 0]
  return { pass: easu.every(item => item.pass) && usm.every(item => item.pass) && srUsm.every(item => item.pass) && gates.every(Boolean), easu, usm, srUsm, noiseEvidence, detailEvidence, haloEvidence, strongHaloEvidence, bypass, casUnchanged }
}

try {
  document.getElementById('result').textContent = JSON.stringify(runPixelChecks())
}
catch (error) {
  document.getElementById('result').textContent = JSON.stringify({ pass: false, error: String(error) })
}
