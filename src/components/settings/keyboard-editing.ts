export type KeyboardEdit = { type: 'insert', text: string } | { type: 'backspace' } | { type: 'clear' }

/** 按 Unicode 字符删除，保留粘贴文本中的完整字符。 */
export function editKeyboardValue(value: string, edit: KeyboardEdit, replace = false): string {
  if (edit.type === 'clear')
    return ''
  if (edit.type === 'backspace')
    return Array.from(value).slice(0, -1).join('')
  return `${replace ? '' : value}${edit.text}`
}
