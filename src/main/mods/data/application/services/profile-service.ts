import Store from 'electron-store'
import type { DataSessionContext, DataUserProfile, XboxWebApiClient } from '../../domain/types'
import { XboxWebApiResilience } from '../../infrastructure/xbox-webapi-resilience'
import { getMainStore, STORE_KEYS } from '../../../../store'

interface ProfileSetting {
  id?: string
  value?: string
}

interface ProfileResponse {
  profileUsers?: Array<{
    settings?: ProfileSetting[]
  }>
}

/**
 * 档案服务
 * - 负责同步并持久化用户资料 settings，并生成基础展示字段
 */
export class ProfileService {
  private readonly store: Store
  private readonly resilience: XboxWebApiResilience

  constructor(store?: Store, resilience?: XboxWebApiResilience) {
    this.store = store ?? getMainStore()
    this.resilience = resilience ?? new XboxWebApiResilience()
  }

  async refreshProfile(_session: DataSessionContext, webApi: XboxWebApiClient): Promise<void> {
    const response = await this.resilience.run('profile.getCurrentUser', async () => {
      return (await webApi.providers.profile.getCurrentUser()) as {
        data?: ProfileResponse
      }
    })

    const settings = response.data?.profileUsers?.[0]?.settings ?? []
    const profilePatch: Record<string, string> = settings.reduce(
      (acc, setting) => {
        if (typeof setting.id === 'string' && typeof setting.value === 'string') {
          acc[setting.id] = setting.value
        }
        return acc
      },
      {} as Record<string, string>
    )

    this.store.set(STORE_KEYS.DATA.PROFILE_CACHE, profilePatch)
  }

  clearCachedProfile(): void {
    this.store.delete(STORE_KEYS.DATA.PROFILE_CACHE)
  }

  getCachedProfile(appLevel: number): DataUserProfile {
    const settings = this.store.get(STORE_KEYS.DATA.PROFILE_CACHE, {}) as Record<string, string>

    // profile provider 当前公开的 settings 字段见 xbox-webapi-guide/profile.md
    const gameDisplayName = settings['GameDisplayName'] ?? ''
    const gameDisplayPicRaw = settings['GameDisplayPicRaw'] ?? ''
    const gamertag = settings['Gamertag'] ?? ''
    const gamerscore = settings['Gamerscore'] ?? ''
    return {
      signedIn: gamertag !== '',
      gameDisplayName,
      gameDisplayPicRaw,
      gamertag,
      gamerscore,
      settings,
      appLevel
    }
  }
}
