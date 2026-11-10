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
    const vertex = compile(gl.VERTEX_SHADER, source === currentSharpen || source.includes('uniform highp sampler2D easuTex') ? FULLSCREEN_VERTEX_SOURCE : '#version 300 es\nlayout(location=0) in vec4 position;void main(){gl_Position=position;}')
    const fragment = compile(gl.FRAGMENT_SHADER, source)
    const program = gl.createProgram()
    gl.attachShader(program, vertex)
    gl.attachShader(program, fragment)
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(program))
    gl.detachShader(program, vertex); gl.detachShader(program, fragment); gl.deleteShader(vertex); gl.deleteShader(fragment)
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
    gl.uniform2f(gl.getUniformLocation(program, 'sourceTexelSize'), 1 / width, 1 / height)
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
  const matrix = []
  const inputs = [
    ['black', pattern(9, 7, () => [0, 0, 0])],
    ['near-black', pattern(9, 7, (x, y) => [x % 3, y % 3, (x+y)%3])],
    ['orientation', pattern(9, 7, (x, y) => y < 3 ? [200, x*10, 10] : [10, x*10, 200])],
    ['text', pattern(9, 7, (x, y) => x%3===0 || y%3===0 ? [255,255,255] : [12,12,12])],
    ['color-lines', pattern(9, 7, (x, y) => [x%2*255,y%2*255,(x+y)%2*255])],
    ['noise', pattern(9, 7, (x, y) => [100+(x*7+y*11)%9,100+(x*5+y*7)%9,100+(x*3+y*5)%9])],
  ]
  for (const [name, input] of inputs) {
    for (const [ow,oh] of [[9,7],[27,21],[5,4]]) {
      for (const filter of [1,2]) for (const strength of [0,0.6,1]) {
        const before = render(baselineSharpen,input,9,7,ow,oh,strength,filter)
        const after = render(currentSharpen,input,9,7,ow,oh,strength,filter)
        const delta = difference(before,after)
        matrix.push({name,output:[ow,oh],filter,strength,...delta,pass:delta.maxCodeValue<=1})
      }
      if (ow>9) {
        const before = render(baselineEasu(true),input,9,7,ow,oh)
        const after = render(buildEasuFragmentSource(true),input,9,7,ow,oh)
        for (const filter of [1,2]) {
          const delta = difference(render(baselineSharpen,before,ow,oh,ow,oh,0.6,filter),render(currentSharpen,after,ow,oh,ow,oh,0.6,filter))
          matrix.push({name,output:[ow,oh],filter,path:'EASU+sharpen',...delta,pass:delta.maxCodeValue<=1})
        }
      }
    }
  }
  return {pass:matrix.every(row=>row.pass),cases:matrix.length,maxCodeValue:Math.max(...matrix.map(row=>row.maxCodeValue)),matrix}
}
try { document.getElementById('result').textContent = JSON.stringify(runPixelChecks()) }
catch(error) { document.getElementById('result').textContent = JSON.stringify({pass:false,error:String(error)}) }
