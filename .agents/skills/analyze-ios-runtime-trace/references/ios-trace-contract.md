# iOS Runtime Trace Contract

## Envelope

Every completed JSONL row uses schema v3 fields:

- `schemaVersion`: `3`
- `seq`: process-local sequence, strictly increasing within `sessionId`
- `tsMs`: Unix epoch milliseconds
- `traceMode` / `traceProfile`: `production` or `dev`
- `dimension`: `core`, `lifecycle`, `network`, `recovery`, `media_supply`, `presentation`, `input`, `native_video`, `frontend`, `engine_log`
- `importance`: `essential`, `key`, `debug`, `raw`
- `category`: `event`, `decision`, `state`, `snapshot`, `log`
- `domain`, `event`, `sessionId`, `payload`
- `payload.platform`: `ios`

## Writer Budgets

| Profile | File size | Retention | Recorded importance |
| --- | ---: | ---: | --- |
| `off` | 0 | 0 | none |
| `production` | 8 MiB | 4 files | essential/key |
| `dev` | 32 MiB | 6 files | essential/key/debug/raw |

The writer flushes every 40 ms or 128 rows. The pending-row pressure threshold is 4096. Under pressure it drops debug/raw rows and emits `traceBudgetNotice` at most once per 60 seconds.

Raw files follow `runtime-trace-ios-<tsMs>-<fileId>.jsonl`. Aggregate exports follow `XBXRC-iOS-Trace-*.jsonl`; their combined size is outside the per-file gate.

## Required Flow Anchors

### Startup

- `appLaunchStarted`
- `authRestoreStarted`
- one of `authRestoreSucceeded`, `authRestoreFailed`

### Library

- `libraryPageAppeared`
- `cloudAccessBoundaryStarted`
- one of `cloudAccessBoundarySucceeded`, `cloudAccessBoundaryFailed`
- `cacheRestoreStarted`
- one cache outcome
- `catalogRefreshStarted`
- one catalog outcome
- one of `skeletonPresented`, `contentPresented`

Conditional flows only require an outcome after their start event appears:

- metadata page
- image candidate
- user refresh
- achievements, playtime, and game-library boundaries

## Privacy Gate

Reject raw values for keys containing token, seed, JWK, handle, OAuth, authorization, callback URL, account ID, XUID, XID, UHS, or refresh code. Boolean presence flags remain valid.

Reject raw HTTP(S) URLs, `ms-xal-*` callbacks, bearer values, GS tokens, refresh tokens, and `cloud-<hex identity>` strings anywhere in serialized rows.

Production `productId` must be fingerprinted. `streamTitleId` and `xboxTitleId` must be fingerprinted in every profile. A fingerprint is a 16-character lowercase hexadecimal string in the current writer.

## Interpretation

- `operationId` correlates one asynchronous boundary or state transition.
- `generation` identifies refresh ownership and stale-result rejection.
- `pageIndex` identifies progressive metadata hydration.
- `traceBudgetNotice` proves debug/raw loss; it does not invalidate essential/key evidence.
- A missing terminal event after a start event is a pairing violation or an abruptly terminated process.
