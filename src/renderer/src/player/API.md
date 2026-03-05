# PlayerClient API

## Entry

```ts
import { PlayerClient } from './new'
```

## Create

```ts
const client = new PlayerClient({
  container: 'player-root',
  uiSystem: [10, 19, 31, 27, 32, -41],
  uiVersion: [0, 2, 0],
  input: {
    pollingRate: 250
  },
  audio: {
    enableAudioControl: true,
    volume: 1
  },
  renderer: {
    enabled: false,
    mode: 'webgl2',
    sharpness: 2,
    format: 'Contain'
  },
  transport: {
    maxVideoBitrateKbps: 0,
    maxAudioBitrateKbps: 0,
    forceMonoAudio: false
  }
})
```

## Lifecycle

```ts
client.bind({
  turnServer: {
    url: 'turn:example.com:3478',
    username: 'user',
    credential: 'pass'
  }
})

const offer = await client.createOffer()
await client.setRemoteDescription(answerSdp)
await client.addIceCandidates(remoteCandidates)
```

## Helper Methods

- `waitForIceCandidates(timeoutMs?)`

### `client.input()`

- `updateConfig(partial)`
- `setGamepadState(state)`
- `pressButton(button, durationMs)`
- `pressButtonStart(button)`
- `pressButtonEnd(button)`
- `moveLeftStick(x, y)`
- `moveRightStick(x, y)`

### `client.audio()`

- `updateConfig(partial)`
- `setVolumeDirect(value)`
- `startMic()`
- `stopMic()` (async renegotiation)
- `getMicState()`

### `client.stats()`

- `snapshot()`
- `subscribe(listener)`

## Core Methods

- `bind(params?)`
- `reset()`
- `close()`
- `events()`
- `getState()`
- `createOffer()`
- `setRemoteDescription(answerSdp)`
- `addIceCandidates(candidates)`

## Events

- `session.stateChanged`
- `transport.iceCandidate`
- `transport.connectionState`
- `transport.track`
- `channel.message`
- `stats.updated`
- `stats.fps`
- `stats.inputPacket`
- `stats.videoFrameProcessed`
- `media.videoReady`
- `media.audioReady`
- `chat.stateChanged`
- `error`
