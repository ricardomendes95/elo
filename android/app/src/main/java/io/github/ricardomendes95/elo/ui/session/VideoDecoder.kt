package io.github.ricardomendes95.elo.ui.session

import android.graphics.Bitmap
import android.graphics.Rect
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.util.Log
import android.view.Surface
import io.github.ricardomendes95.elo.core.VideoSink
import java.nio.ByteBuffer
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/**
 * Decodifica o vídeo da sessão num [Surface] (o da `TextureView`).
 *
 * - H.264: o host manda unidades de acesso **Annex-B**, um quadro por mensagem
 *   (`key` = IDR com SPS/PPS). O decoder `video/avc` do `MediaCodec` é configurado
 *   no primeiro keyframe (SPS/PPS também vão em `csd-0`/`csd-1`) e reconfigurado
 *   quando o tamanho muda (troca de monitor/resolução).
 * - BGRA (codec de tiles, quando o host não tem H.264): desenhado com
 *   `lockCanvas` no mesmo Surface (com o decoder liberado).
 *
 * Tudo roda numa thread própria: o callback do Rust só enfileira.
 */
class VideoDecoder(private val surface: Surface) : VideoSink {
    private sealed interface Job {
        class H264(val w: Int, val h: Int, val key: Boolean, val data: ByteArray) : Job
        class Bgra(val w: Int, val h: Int, val data: ByteArray) : Job
        data object Stop : Job
    }

    private val queue = LinkedBlockingQueue<Job>()
    private var codec: MediaCodec? = null
    private var codedW = 0
    private var codedH = 0
    private var waitingKey = true
    private var pts = 0L
    private var bitmap: Bitmap? = null
    private var pixels: IntArray? = null
    private val thread = Thread(::loop, "elo-video").apply { start() }

    /** Chamado a cada quadro exibido (para contar FPS na UI). */
    @Volatile var onFrame: (() -> Unit)? = null

    override fun onH264(width: Int, height: Int, key: Boolean, data: ByteArray) {
        // Se a fila cresceu demais (decoder lento), descarta até o próximo keyframe.
        if (queue.size > 60 && !key) return
        if (key && queue.size > 60) queue.clear()
        queue.offer(Job.H264(width, height, key, data))
    }

    override fun onBgra(width: Int, height: Int, bgra: ByteArray) {
        if (queue.size > 4) return // só o quadro mais recente importa
        queue.offer(Job.Bgra(width, height, bgra))
    }

    fun release() {
        queue.clear()
        queue.offer(Job.Stop)
        thread.join(1500)
    }

    private fun loop() {
        try {
            while (true) {
                when (val j = queue.poll(20, TimeUnit.MILLISECONDS)) {
                    null -> drain(0)
                    is Job.Stop -> break
                    is Job.H264 -> decode(j)
                    is Job.Bgra -> drawBgra(j)
                }
            }
        } catch (_: InterruptedException) {
        } finally {
            stopCodec()
        }
    }

    private fun decode(j: Job.H264) {
        val w = (j.w + 1) and 1.inv()
        val h = (j.h + 1) and 1.inv()
        if (codec == null || w != codedW || h != codedH) {
            if (!j.key) return // espera um keyframe para (re)configurar
            stopCodec()
            if (!startCodec(w, h, j.data)) return
        }
        if (waitingKey && !j.key) return
        waitingKey = false
        val c = codec ?: return
        try {
            val idx = c.dequeueInputBuffer(50_000)
            if (idx < 0) {
                drain(0)
                return
            }
            val buf: ByteBuffer = c.getInputBuffer(idx) ?: return
            buf.clear()
            if (buf.capacity() < j.data.size) {
                c.queueInputBuffer(idx, 0, 0, 0, 0)
                return
            }
            buf.put(j.data)
            pts += 16_666
            c.queueInputBuffer(idx, 0, j.data.size, pts, if (j.key) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
            drain(0)
        } catch (e: Exception) {
            Log.w(TAG, "erro no decoder; reiniciando no próximo keyframe", e)
            stopCodec()
        }
    }

    private fun drain(timeoutUs: Long) {
        val c = codec ?: return
        val info = MediaCodec.BufferInfo()
        try {
            while (true) {
                val out = c.dequeueOutputBuffer(info, timeoutUs)
                if (out >= 0) {
                    c.releaseOutputBuffer(out, true)
                    onFrame?.invoke()
                } else if (out == MediaCodec.INFO_TRY_AGAIN_LATER) {
                    break
                } // INFO_OUTPUT_FORMAT_CHANGED / BUFFERS_CHANGED: segue drenando
            }
        } catch (e: Exception) {
            Log.w(TAG, "erro ao drenar o decoder", e)
            stopCodec()
        }
    }

    private fun startCodec(w: Int, h: Int, keyframe: ByteArray): Boolean {
        return try {
            val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h)
            val (sps, pps) = findParamSets(keyframe)
            if (sps != null && pps != null) {
                fmt.setByteBuffer("csd-0", ByteBuffer.wrap(sps))
                fmt.setByteBuffer("csd-1", ByteBuffer.wrap(pps))
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) fmt.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            fmt.setInteger(MediaFormat.KEY_PRIORITY, 0) // tempo real
            val c = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            c.configure(fmt, surface, null, 0)
            c.start()
            codec = c
            codedW = w
            codedH = h
            waitingKey = true
            Log.i(TAG, "decoder H.264 ${w}x$h (${c.name})")
            true
        } catch (e: Exception) {
            Log.e(TAG, "não foi possível iniciar o decoder H.264", e)
            stopCodec()
            false
        }
    }

    private fun stopCodec() {
        codec?.let {
            runCatching { it.stop() }
            runCatching { it.release() }
        }
        codec = null
        codedW = 0
        codedH = 0
    }

    private fun drawBgra(j: Job.Bgra) {
        if (codec != null) stopCodec()
        val n = j.w * j.h
        if (j.data.size < n * 4) return
        var bmp = bitmap
        if (bmp == null || bmp.width != j.w || bmp.height != j.h) {
            bmp = Bitmap.createBitmap(j.w, j.h, Bitmap.Config.ARGB_8888)
            bitmap = bmp
            pixels = IntArray(n)
        }
        val px = pixels!!
        val d = j.data
        var i = 0
        var o = 0
        while (o < n) {
            val b = d[i].toInt() and 0xff
            val g = d[i + 1].toInt() and 0xff
            val r = d[i + 2].toInt() and 0xff
            px[o] = (0xff shl 24) or (r shl 16) or (g shl 8) or b
            i += 4
            o++
        }
        bmp.setPixels(px, 0, j.w, 0, 0, j.w, j.h)
        try {
            val canvas = surface.lockCanvas(null) ?: return
            try {
                canvas.drawBitmap(bmp, null, Rect(0, 0, canvas.width, canvas.height), null)
            } finally {
                surface.unlockCanvasAndPost(canvas)
            }
            onFrame?.invoke()
        } catch (e: Exception) {
            Log.w(TAG, "falha ao desenhar quadro BGRA", e)
        }
    }

    companion object {
        private const val TAG = "EloVideo"

        /** Extrai SPS (NAL 7) e PPS (NAL 8), com start code, de uma unidade Annex-B. */
        fun findParamSets(au: ByteArray): Pair<ByteArray?, ByteArray?> {
            var sps: ByteArray? = null
            var pps: ByteArray? = null
            val starts = ArrayList<Int>() // índice do primeiro byte do start code
            val payloads = ArrayList<Int>() // índice do byte do cabeçalho NAL
            var i = 0
            while (i + 3 <= au.size) {
                if (au[i].toInt() == 0 && au[i + 1].toInt() == 0) {
                    if (au[i + 2].toInt() == 1) {
                        starts.add(i); payloads.add(i + 3); i += 3; continue
                    }
                    if (i + 4 <= au.size && au[i + 2].toInt() == 0 && au[i + 3].toInt() == 1) {
                        starts.add(i); payloads.add(i + 4); i += 4; continue
                    }
                }
                i++
            }
            for (k in payloads.indices) {
                val p = payloads[k]
                if (p >= au.size) continue
                val end = if (k + 1 < starts.size) starts[k + 1] else au.size
                val type = au[p].toInt() and 0x1f
                val nal = byteArrayOf(0, 0, 0, 1) + au.copyOfRange(p, end)
                if (type == 7 && sps == null) sps = nal
                if (type == 8 && pps == null) pps = nal
            }
            return sps to pps
        }
    }
}
