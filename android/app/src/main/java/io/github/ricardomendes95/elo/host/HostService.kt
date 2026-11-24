package io.github.ricardomendes95.elo.host

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import io.github.ricardomendes95.elo.MainActivity
import io.github.ricardomendes95.elo.core.EloCore
import io.github.ricardomendes95.elo.data.Prefs
import io.github.ricardomendes95.elo.ffi.Host
import io.github.ricardomendes95.elo.ffi.HostInput
import io.github.ricardomendes95.elo.ffi.HostListener
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Compartilha a tela: `MediaProjection` → `VirtualDisplay` → `ImageReader` (RGBA_8888,
 * no máx. ~1280 px no lado maior, ~15 qps) → `Host.pushFrame`. Também liga o host
 * Rust (LAN + P2P) e mostra a notificação "Compartilhando a tela" com a ação Parar.
 */
class HostService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var startJob: Job? = null
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var reader: ImageReader? = null
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var host: Host? = null
    private var capW = 0
    private var capH = 0
    private var lastPush = 0L
    private var stopping = false

    private class Frame(val bytes: ByteArray, val w: Int, val h: Int, val stride: Int) {
        fun pushTo(h: Host) = h.pushFrame(bytes, w.toUInt(), this.h.toUInt(), stride.toUInt())
    }
    @Volatile private var lastFrame: Frame? = null
    private var displayListener: DisplayManager.DisplayListener? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> { stopSharing(); return START_NOT_STICKY }
            ACTION_APPROVE -> { HostState.respondApproval(true); return START_NOT_STICKY }
            ACTION_DENY -> { HostState.respondApproval(false); return START_NOT_STICKY }
            ACTION_START -> {
                if (projection != null) return START_NOT_STICKY
                startSharing(intent)
            }
            else -> if (projection == null) { stopSelf(); return START_NOT_STICKY }
        }
        return START_NOT_STICKY
    }

    // ---- início ----

    private fun startSharing(intent: Intent) {
        ensureChannels()
        val n = ongoingNotification("Compartilhando a tela")
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIF_ONGOING, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            } else {
                startForeground(NOTIF_ONGOING, n)
            }
        } catch (e: Exception) {
            fail("Não foi possível iniciar o serviço: ${e.message}")
            return
        }
        val code = intent.getIntExtra(EXTRA_RESULT_CODE, 0)
        @Suppress("DEPRECATION")
        val data = intent.getParcelableExtra<Intent>(EXTRA_RESULT_DATA)
        if (data == null) { fail("Permissão de captura de tela ausente."); return }
        val mpm = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        val mp = try { mpm.getMediaProjection(code, data) } catch (e: Exception) { null }
        if (mp == null) { fail("O Android recusou a captura de tela."); return }
        projection = mp
        thread = HandlerThread("elo-capture").also { it.start() }
        handler = Handler(thread!!.looper)
        // Obrigatório no Android 14+ (antes de criar o VirtualDisplay).
        mp.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() { Handler(Looper.getMainLooper()).post { stopSharing() } }
        }, handler)

        HostState.setError(null)
        HostState.setRunning(true)
        HostState.setStatus("Iniciando…")
        startCapture(mp)

        startJob = scope.launch {
            try {
                val h = EloCore.startHost(Prefs.deviceName.value, Prefs.hostPassword.value, Listener())
                if (stopping) { h.stop(); return@launch }
                host = h
                HostState.host = h
                lastFrame?.pushTo(h)
                HostState.setAddress(h.address())
            } catch (e: Throwable) {
                Log.w(TAG, "start_host falhou", e)
                Handler(Looper.getMainLooper()).post { fail("Não foi possível iniciar o host: ${e.message}") }
            }
        }
    }

    private fun startCapture(mp: MediaProjection) {
        val (w, h) = captureSize()
        capW = w; capH = h
        val dpi = resources.displayMetrics.densityDpi
        val r = newReader(w, h)
        reader = r
        display = mp.createVirtualDisplay(
            "elo-share", w, h, dpi, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            r.surface, null, handler,
        )
        // Rotação / mudança de resolução: recria o leitor e redimensiona o display virtual.
        val dm = getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
        val l = object : DisplayManager.DisplayListener {
            override fun onDisplayAdded(id: Int) {}
            override fun onDisplayRemoved(id: Int) {}
            override fun onDisplayChanged(id: Int) {
                if (id != android.view.Display.DEFAULT_DISPLAY) return
                val (nw, nh) = captureSize()
                if (nw == capW && nh == capH) return
                capW = nw; capH = nh
                val old = reader
                val nr = newReader(nw, nh)
                reader = nr
                display?.resize(nw, nh, resources.displayMetrics.densityDpi)
                display?.surface = nr.surface
                old?.close()
            }
        }
        displayListener = l
        dm.registerDisplayListener(l, Handler(Looper.getMainLooper()))
    }

    /** Tamanho do quadro: tela real reduzida para no máx. ~1280 px no lado maior. */
    private fun captureSize(): Pair<Int, Int> {
        val (rw, rh) = HostState.realScreenSize(this)
        val scale = min(1f, MAX_SIDE / max(rw, rh).toFloat())
        return max(2, (rw * scale).roundToInt()) to max(2, (rh * scale).roundToInt())
    }

    private val trailing = Runnable { trailingScheduled = false; pushLatest() }
    private var trailingScheduled = false

    private fun newReader(w: Int, h: Int): ImageReader {
        val r = ImageReader.newInstance(w, h, PixelFormat.RGBA_8888, 2)
        r.setOnImageAvailableListener({ rd ->
            val wait = FRAME_INTERVAL_MS - (System.nanoTime() / 1_000_000 - lastPush)
            if (wait <= 0) {
                pushLatest(rd)
            } else if (!trailingScheduled) {
                // Não descarta: o último quadro de uma tela parada precisa chegar.
                trailingScheduled = true
                handler?.postDelayed(trailing, wait)
            }
        }, handler)
        return r
    }

    private fun pushLatest(rd: ImageReader? = reader) {
        val img = try { rd?.acquireLatestImage() } catch (e: Exception) { null } ?: return
        try {
            lastPush = System.nanoTime() / 1_000_000
            val plane = img.planes[0]
            val buf = plane.buffer
            val bytes = ByteArray(buf.remaining())
            buf.get(bytes)
            HostState.frameSize = img.width to img.height
            val f = Frame(bytes, img.width, img.height, plane.rowStride)
            lastFrame = f
            host?.let { f.pushTo(it) }
        } catch (e: Exception) {
            Log.w(TAG, "quadro descartado", e)
        } finally {
            img.close()
        }
    }

    // ---- fim ----

    private fun fail(msg: String) {
        HostState.setError(msg)
        stopSharing()
    }

    private fun stopSharing() {
        if (stopping) return
        stopping = true
        HostState.respondApproval(false)
        (getSystemService(Context.DISPLAY_SERVICE) as DisplayManager).let { dm ->
            displayListener?.let { dm.unregisterDisplayListener(it) }
        }
        displayListener = null
        val h = host
        host = null
        HostState.host = null
        scope.launch { runCatching { h?.stop() } }
        runCatching { display?.release() }
        runCatching { reader?.close() }
        runCatching { projection?.stop() }
        display = null; reader = null; projection = null
        thread?.quitSafely(); thread = null
        HostState.setRunning(false)
        HostState.frameSize = 0 to 0
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).cancel(NOTIF_APPROVAL)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (!stopping) stopSharing()
        scope.cancel()
        super.onDestroy()
    }

    // ---- notificações ----

    private fun ensureChannels() {
        val nm = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(CH_ONGOING, "Compartilhamento de tela", NotificationManager.IMPORTANCE_LOW),
        )
        nm.createNotificationChannel(
            NotificationChannel(CH_APPROVAL, "Pedidos de acesso", NotificationManager.IMPORTANCE_HIGH),
        )
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_NEW_TASK),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun action(act: String, req: Int): PendingIntent = PendingIntent.getService(
        this, req, Intent(this, HostService::class.java).setAction(act),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun ongoingNotification(text: String): Notification =
        NotificationCompat.Builder(this, CH_ONGOING)
            .setSmallIcon(android.R.drawable.ic_menu_share)
            .setContentTitle("Compartilhando a tela")
            .setContentText(text)
            .setOngoing(true)
            .setContentIntent(openApp())
            .addAction(0, "Parar", action(ACTION_STOP, 1))
            .build()

    private fun updateOngoing(text: String) {
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).notify(NOTIF_ONGOING, ongoingNotification(text))
    }

    private fun showApproval(req: ApprovalRequest) {
        if (HostState.uiVisible) return
        val n = NotificationCompat.Builder(this, CH_APPROVAL)
            .setSmallIcon(android.R.drawable.ic_menu_share)
            .setContentTitle("Pedido de acesso")
            .setContentText("${req.name} quer ver e controlar este celular.")
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .addAction(0, "Aceitar", action(ACTION_APPROVE, 2))
            .addAction(0, "Recusar", action(ACTION_DENY, 3))
            .build()
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).notify(NOTIF_APPROVAL, n)
    }

    // ---- eventos do host Rust (threads de fundo) ----

    private inner class Listener : HostListener {
        override fun onApproval(name: String, fingerprint: String): Boolean =
            HostState.awaitApproval(
                ApprovalRequest(name, fingerprint),
                onShown = { showApproval(it) },
                onDone = { (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).cancel(NOTIF_APPROVAL) },
            )

        override fun onSessionStarted(peer: String) {
            HostState.setPeer(peer)
            HostState.setStatus("Em sessão com $peer")
            updateOngoing("Sendo controlado por $peer")
        }

        override fun onSessionEnded() {
            HostState.setPeer(null)
            updateOngoing("Aguardando conexões")
        }

        override fun onInput(ev: HostInput) {
            EloAccessibilityService.dispatch(ev)
        }

        override fun onStatus(text: String) {
            if (HostState.peer.value == null) HostState.setStatus(text)
        }
    }

    companion object {
        private const val TAG = "EloHost"
        const val ACTION_START = "io.github.ricardomendes95.elo.host.START"
        const val ACTION_STOP = "io.github.ricardomendes95.elo.host.STOP"
        const val ACTION_APPROVE = "io.github.ricardomendes95.elo.host.APPROVE"
        const val ACTION_DENY = "io.github.ricardomendes95.elo.host.DENY"
        const val EXTRA_RESULT_CODE = "resultCode"
        const val EXTRA_RESULT_DATA = "resultData"
        private const val CH_ONGOING = "elo_share"
        private const val CH_APPROVAL = "elo_approval"
        private const val NOTIF_ONGOING = 41
        private const val NOTIF_APPROVAL = 42
        private const val MAX_SIDE = 1280f
        private const val FRAME_INTERVAL_MS = 66L

        fun start(ctx: Context, resultCode: Int, data: Intent) {
            val i = Intent(ctx, HostService::class.java).setAction(ACTION_START)
                .putExtra(EXTRA_RESULT_CODE, resultCode).putExtra(EXTRA_RESULT_DATA, data)
            ctx.startForegroundService(i)
        }

        fun stop(ctx: Context) {
            if (HostState.running.value) ctx.startService(Intent(ctx, HostService::class.java).setAction(ACTION_STOP))
        }
    }
}
