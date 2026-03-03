export class MediaSourceFactory {
    createVideoMediaSource(): { url: string; mediaSource: MediaSource } {
        const mediaSource = new MediaSource()
        const url = URL.createObjectURL(mediaSource)
        mediaSource.addEventListener('sourceopen', () => {
            const videoSourceBuffer = mediaSource.addSourceBuffer('video/mp4; codecs="avc1.42c020"')
            videoSourceBuffer.mode = 'sequence'
        })
        return { url, mediaSource }
    }

    createAudioMediaSource(): { url: string; mediaSource: MediaSource } {
        const mediaSource = new MediaSource()
        const url = URL.createObjectURL(mediaSource)
        mediaSource.addEventListener('sourceopen', () => {
            const codec = navigator.userAgent.search('Safari') >= 0 && navigator.userAgent.search('Chrome') < 0
                ? 'audio/mp4'
                : 'audio/webm;codecs=opus'
            const audioSourceBuffer = mediaSource.addSourceBuffer(codec)
            audioSourceBuffer.mode = 'sequence'
        })
        return { url, mediaSource }
    }
}
