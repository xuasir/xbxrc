export interface QueryResult<T> { accountId: string, targetId: string, updatedAt: number, data: T }
export interface LibraryPage<T> { items: T[], continuation: string | null }
export interface GameProgress { unlockedCount: number | null, totalCount: number | null, earnedGamerscore: number | null, totalGamerscore: number | null }
export interface LibraryGame { description: string | null, xboxTitleId: string, scid: string | null, productId: string | null, name: string, artworkUrl: string | null, lastPlayedAt: string | null, progress: GameProgress | null }
export interface LibraryAchievement { id: string, scid: string | null, name: string, description: string, lockedDescription: string, imageUrl: string | null, isSecret: boolean, isUnlocked: boolean, gamerscore: number | null, progressPercentage: number | null, requirements: { id: string, current: string | null, target: string | null }[], unlockedAt: string | null }
export interface LibraryPerson { xuid: string, displayName: string, gamertag: string, avatarUrl: string | null, gamerscore: string | null, presenceState: string | null, presenceText: string | null, currentTitleId: string | null, isFollowedByCaller: boolean | null, isFollowingCaller: boolean | null }
export interface PersonalAccount { profile: LibraryPerson | null, friends: LibraryPerson[] | null, notices: string[] }
export interface Playtime { xboxTitleId: string, minutes: number | null, error: string | null }
export interface InstalledGame { id: string, xboxTitleId: string | null, productId: string | null, name: string, artworkUrl?: string | null, storageDeviceId: string | null, sizeBytes: number | null, installedAt: string | null, lastPlayedAt: string | null }
export interface ConsoleStorage { id: string, name: string, freeBytes: number | null, totalBytes: number | null }
export interface ConsoleLibrary { consoleId: string, games: InstalledGame[] | null, storage: ConsoleStorage[] | null, powerState: string | null, notices: string[] }
export type MediaKind = 'screenshot' | 'clip'
export interface LibraryMedia { id: string, kind: MediaKind, xboxTitleId: string | null, titleName: string, capturedAt: string | null, durationSeconds: number | null, thumbnailUrl: string | null, url: string | null, expiresAt: string | null }
