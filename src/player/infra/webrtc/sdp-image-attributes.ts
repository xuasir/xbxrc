/** imageattr 的方向关键字分词；send/recv 各自保留标准参数串。 */
export function parseSdpImageAttributes(line: string): { payloadType: string, send?: string, recv?: string } | undefined {
  const prefix = /^a=imageattr:(\d+|\*)[ \t]+/.exec(line)
  if (!prefix)
    return undefined
  const tokens = line.slice(prefix[0].length).trim().split(/\s+/)
  const attributes: { payloadType: string, send?: string, recv?: string } = { payloadType: prefix[1] }
  let direction: 'send' | 'recv' | undefined
  const values: string[] = []
  const finish = () => {
    if (direction && values.length)
      attributes[direction] = values.join(' ')
    values.length = 0
  }
  for (const token of tokens) {
    if (token === 'send' || token === 'recv') {
      finish()
      direction = token
    }
    else if (direction) {
      values.push(token)
    }
  }
  finish()
  return attributes
}
