import { describe, expect, it } from 'vitest'
import { editKeyboardValue } from './keyboard-editing'

describe('controller text entry', () => {
  it('replaces a numeric value on the first insertion and appends subsequent digits', () => {
    const draft = editKeyboardValue('20', { type: 'insert', text: '1' }, true)
    expect(editKeyboardValue(draft, { type: 'insert', text: '5' })).toBe('15')
  })
  it('preserves punctuation, case and spaces used in relay credentials', () => {
    const draft = editKeyboardValue('turn:relay.test', { type: 'insert', text: ':3478?transport=udp' })
    expect(draft).toBe('turn:relay.test:3478?transport=udp')
    expect(editKeyboardValue('AbC!@', { type: 'insert', text: ' 9' })).toBe('AbC!@ 9')
  })
  it('deletes a complete unicode character and handles empty and cleared drafts', () => {
    expect(editKeyboardValue('中文🎮', { type: 'backspace' })).toBe('中文')
    expect(editKeyboardValue('', { type: 'backspace' })).toBe('')
    expect(editKeyboardValue('secret', { type: 'clear' })).toBe('')
  })
})
