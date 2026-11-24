package io.github.ricardomendes95.elo.ui.session

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.util.Log

/**
 * Toca o áudio do host: PCM s16le intercalado (hoje 24 kHz mono, blocos de 20 ms)
 * num [AudioTrack] em `MODE_STREAM`. Escreve sem bloquear e descarta blocos quando
 * a fila passa de ~250 ms, para não acumular latência (como o desktop).
 */
class AudioOut {
    private var track: AudioTrack? = null
    private var rate = 0
    private var channels = 0

    @Synchronized
    fun push(rate: Int, channels: Int, pcm: ByteArray) {
        if (track == null || rate != this.rate || channels != this.channels) {
            release()
            open(rate, channels) ?: return
        }
        val t = track ?: return
        // Bytes ainda não tocados ≈ escritos − tocados.
        val frameBytes = 2 * channels
        val pending = written - t.playbackHeadPosition.toLong() * frameBytes
        val maxPending = rate.toLong() * frameBytes / 4 // 250 ms
        if (pending > maxPending) return
        val n = t.write(pcm, 0, pcm.size, AudioTrack.WRITE_NON_BLOCKING)
        if (n > 0) written += n
    }

    private var written = 0L

    private fun open(rate: Int, channels: Int): AudioTrack? {
        return try {
            val mask = if (channels == 1) AudioFormat.CHANNEL_OUT_MONO else AudioFormat.CHANNEL_OUT_STEREO
            val min = AudioTrack.getMinBufferSize(rate, mask, AudioFormat.ENCODING_PCM_16BIT)
            val t = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(rate)
                        .setChannelMask(mask)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(maxOf(min, rate * 2 * channels / 2)) // ~500 ms de espaço
                .build()
            t.play()
            track = t
            this.rate = rate
            this.channels = channels
            written = 0
            t
        } catch (e: Exception) {
            Log.w("EloAudio", "AudioTrack indisponível", e)
            null
        }
    }

    @Synchronized
    fun release() {
        track?.let {
            runCatching { it.pause(); it.flush(); it.stop() }
            it.release()
        }
        track = null
        rate = 0
        channels = 0
    }
}
