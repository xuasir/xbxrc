export const UPDATER_PROGRESS_CHANNEL = 'updater://progress'

export type UpdaterProgressEvent
  = | {
    event: 'started'
    contentLength: number | null
  }
  | {
    event: 'progress'
    downloaded: number
    contentLength: number | null
  }
  | {
    event: 'finished'
  }
