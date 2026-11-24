package io.github.ricardomendes95.elo.host

import android.content.Context
import android.graphics.Point
import android.os.Build
import android.view.WindowManager
import io.github.ricardomendes95.elo.ffi.Host
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.net.Inet4Address
import java.net.NetworkInterface
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/** Pedido de acesso aguardando resposta do dono do celular. */
data class ApprovalRequest(val name: String, val fingerprint: String)

/** Estado do papel de host, compartilhado entre o serviço, o serviço de acessibilidade e a UI. */
object HostState {
    private val _running = MutableStateFlow(false)
    /** Compartilhamento ligado (serviço em primeiro plano ativo). */
    val running: StateFlow<Boolean> = _running.asStateFlow()

    private val _address = MutableStateFlow<String?>(null)
    /** ID P2P deste aparelho (disponível depois que o host sobe). */
    val address: StateFlow<String?> = _address.asStateFlow()

    private val _status = MutableStateFlow("")
    val status: StateFlow<String> = _status.asStateFlow()

    private val _peer = MutableStateFlow<String?>(null)
    /** Nome de quem está controlando agora (null = sem sessão). */
    val peer: StateFlow<String?> = _peer.asStateFlow()

    private val _approval = MutableStateFlow<ApprovalRequest?>(null)
    val approval: StateFlow<ApprovalRequest?> = _approval.asStateFlow()

    private val _accessibility = MutableStateFlow(false)
    /** O serviço de acessibilidade está ligado (sem ele a sessão é só visualização). */
    val accessibility: StateFlow<Boolean> = _accessibility.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    /** A tela do Elo está visível (se não, o pedido de acesso também vira notificação). */
    @Volatile var uiVisible = false

    @Volatile internal var host: Host? = null
    /** Tamanho do último quadro enviado (pixels), para mapear coordenadas remotas. */
    @Volatile var frameSize: Pair<Int, Int> = 0 to 0

    private var pending: CompletableFuture<Boolean>? = null

    internal fun setRunning(v: Boolean) { _running.value = v; if (!v) { _address.value = null; _peer.value = null; _status.value = "" } }
    internal fun setAddress(v: String?) { _address.value = v }
    internal fun setStatus(v: String) { _status.value = v }
    internal fun setPeer(v: String?) { _peer.value = v }
    internal fun setAccessibility(v: Boolean) { _accessibility.value = v }
    fun setError(v: String?) { _error.value = v }

    /** Bloqueia até o dono responder (ou ~115 s). Chamado fora da thread de UI. */
    internal fun awaitApproval(req: ApprovalRequest, onShown: (ApprovalRequest) -> Unit, onDone: () -> Unit): Boolean {
        val f = CompletableFuture<Boolean>()
        synchronized(this) { pending?.complete(false); pending = f }
        _approval.value = req
        onShown(req)
        return try {
            f.get(115, TimeUnit.SECONDS)
        } catch (_: Exception) {
            false
        } finally {
            synchronized(this) { if (pending === f) pending = null }
            _approval.value = null
            onDone()
        }
    }

    /** Resposta do dono (diálogo ou notificação). */
    fun respondApproval(allow: Boolean) {
        synchronized(this) { pending?.complete(allow) }
    }

    /** Encerra a sessão em andamento (o host continua ligado). */
    fun endSession() {
        runCatching { host?.endSession() }
    }

    /** IPv4 da rede local (Wi-Fi/Ethernet), ou null. */
    fun lanIp(): String? = runCatching {
        NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback }
            .sortedByDescending { it.name.startsWith("wlan") }
            .flatMap { it.inetAddresses.toList() }
            .firstOrNull { it is Inet4Address && it.isSiteLocalAddress }
            ?.hostAddress
    }.getOrNull()

    /** Tamanho real da tela em pixels (na orientação atual). */
    @Suppress("DEPRECATION")
    fun realScreenSize(ctx: Context): Pair<Int, Int> {
        val wm = ctx.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        return if (Build.VERSION.SDK_INT >= 30) {
            val b = wm.maximumWindowMetrics.bounds
            b.width() to b.height()
        } else {
            val p = Point()
            wm.defaultDisplay.getRealSize(p)
            p.x to p.y
        }
    }
}
