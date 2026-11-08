interface CatalogIdentity {
  scopeKey?: string
  updatedAt?: number
}

export class CatalogRequestState {
  private generation = 0
  private scopeKey: string | undefined
  private updatedAt: number | undefined

  capture(): number { return this.generation }
  isCurrent(generation: number): boolean { return generation === this.generation }

  clear(): void {
    this.generation++
    this.scopeKey = undefined
    this.updatedAt = undefined
  }

  acceptResponse(generation: number, payload: CatalogIdentity): boolean {
    if (!this.isCurrent(generation))
      return false
    if (payload.scopeKey === this.scopeKey && this.isOlder(payload.updatedAt))
      return false
    this.scopeKey = payload.scopeKey
    this.updatedAt = payload.updatedAt
    return true
  }

  acceptEvent(payload: CatalogIdentity): boolean {
    if (!this.scopeKey || payload.scopeKey !== this.scopeKey || this.isOlder(payload.updatedAt))
      return false
    this.updatedAt = payload.updatedAt
    return true
  }

  private isOlder(updatedAt: number | undefined): boolean {
    return this.updatedAt !== undefined && (updatedAt === undefined || updatedAt < this.updatedAt)
  }
}
