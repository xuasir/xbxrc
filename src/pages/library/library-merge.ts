import type { ConsoleLibrary, PersonalAccount } from '@/shared/rpc/library'

export function mergeConsoleLibrary(old: ConsoleLibrary, next: ConsoleLibrary): ConsoleLibrary {
  return { ...next, games: next.games ?? old.games, storage: next.storage ?? old.storage, powerState: next.powerState ?? old.powerState }
}
export function mergePersonalAccount(old: PersonalAccount, next: PersonalAccount): PersonalAccount {
  return { ...next, profile: next.profile ?? old.profile, friends: next.friends ?? old.friends }
}
