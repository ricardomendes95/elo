package io.github.ricardomendes95.elo.core

import android.content.Context
import android.os.Build
import io.github.ricardomendes95.elo.ffi.Discovery
import io.github.ricardomendes95.elo.ffi.Elo
import io.github.ricardomendes95.elo.ffi.EloException
import io.github.ricardomendes95.elo.ffi.Host
import io.github.ricardomendes95.elo.ffi.HostListener
import io.github.ricardomendes95.elo.ffi.Monitor
import io.github.ricardomendes95.elo.ffi.Session
import io.github.ricardomendes95.elo.ffi.SessionInfo
import io.github.ricardomendes95.elo.ffi.SessionListener
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Ponte única entre a UI e o núcleo Rust (`libelo_android.so`, via UniFFI).
 *
 * - [init] uma vez no `Application`/`MainActivity`.
 * - [connect] (suspend, roda em `Dispatchers.IO`) devolve [ConnectResult]; em caso de
 *   sucesso a sessão fica em [active] até terminar.
 * - A tela de sessão observa [ActiveSession] (flows) e registra um [VideoSink].
 */
object EloCore {
    private var elo: Elo? = null
    private lateinit var appContext: Context

    private val _active = MutableStateFlow<ActiveSession?>(null)
    /** Sessão de controle em andamento (null = nenhuma). */
    val active: StateFlow<ActiveSession?> = _active.asStateFlow()

    fun init(context: Context) {
        if (elo != null) return
        appContext = context.applicationContext
        System.setProperty("jna.debug_load", "false")
        elo = Elo(appContext.filesDir.absolutePath)
    }

    private fun core(): Elo = elo ?: error("EloCore.init não foi chamado")

    /** Nome padrão do aparelho, mostrado ao host no pedido de aprovação. */
    fun defaultDeviceName(): String =
        listOfNotNull(Build.MANUFACTURER?.replaceFirstChar { it.uppercase() }, Build.MODEL)
            .joinToString(" ").ifBlank { "Android" }

    /** Pasta onde ficam os arquivos que o host enviar. */
    fun downloadDir(): File =
        File(appContext.getExternalFilesDir(null) ?: appContext.filesDir, "Recebidos").apply { mkdirs() }

    /** Endereço Elo (P2P) deste aparelho. Lento na 1ª vez (liga o nó iroh). */
    suspend fun deviceId(): Result<String> = withContext(Dispatchers.IO) {
        runCatching { core().deviceId() }.mapError()
    }

    /** Liga o papel de host (bloqueante: chame em `Dispatchers.IO`). */
    fun startHost(name: String, password: String, listener: HostListener): Host =
        core().startHost(name.ifBlank { defaultDeviceName() }, password, listener)

    /**
     * Cria um descobridor de hosts na LAN. Chame `start()` e leia `devices()`
     * periodicamente; feche-o (`close()`/`stop()`) ao sair. Segure um
     * `WifiManager.MulticastLock` enquanto estiver ativo (ver [HomeScreen]).
     */
    fun newDiscovery(): Discovery = Discovery()

    /** Presença de um favorito/endereço (até ~8 s). */
    suspend fun isOnline(target: String, lan: Boolean, directory: String): Boolean =
        withContext(Dispatchers.IO) { runCatching { core().isOnline(target, lan, directory) }.getOrDefault(false) }

    /**
     * Conecta a um host. Senha vazia = pedido de aprovação (espera até ~2 min).
     * Se o host tiver 2FA e [totpCode] for nulo, devolve [ConnectResult.NeedTwoFa]:
     * peça o código e chame de novo com ele.
     */
    suspend fun connect(req: ConnectRequest): ConnectResult = withContext(Dispatchers.IO) {
        val listener = ActiveSession.Listener()
        try {
            val s = core().connect(
                req.target.trim(), req.lan, req.password, req.name.ifBlank { defaultDeviceName() },
                req.totpCode, req.directory, downloadDir().absolutePath, listener,
            )
            val a = ActiveSession(s, s.info(), req.label.ifBlank { req.target.trim() }, listener) {
                _active.compareAndSet(it, null)
            }
            listener.owner = a
            _active.value = a
            ConnectResult.Ok(a)
        } catch (e: EloException.TwoFaRequired) {
            ConnectResult.NeedTwoFa
        } catch (e: EloException.Failed) {
            ConnectResult.Error(friendlyError(e.msg))
        } catch (e: Throwable) {
            ConnectResult.Error(e.message ?: e.toString())
        }
    }

    private fun <T> Result<T>.mapError(): Result<T> = recoverCatching {
        throw IllegalStateException(
            when (it) {
                is EloException.Failed -> friendlyError(it.msg)
                else -> it.message ?: it.toString()
            },
        )
    }

    /** Traduz as mensagens técnicas mais comuns (mesma ideia do `friendly_error` do desktop). */
    fun friendlyError(raw: String): String {
        val e = raw.lowercase()
        return when {
            "senha incorreta" in e -> "Senha incorreta, acesso negado ou muitas tentativas."
            "acesso negado" in e -> "O outro lado recusou o pedido (ou não há ninguém para aprovar)."
            "tempo esgotado" in e || "timed out" in e -> "Tempo esgotado: o computador não respondeu."
            "connection refused" in e -> "Conexão recusada: o Elo está aberto no outro computador?"
            "no route" in e || "unreachable" in e -> "Endereço inalcançável."
            "diretório" in e -> raw
            else -> raw
        }
    }
}

data class ConnectRequest(
    /** Endereço Elo, ID curto (9 dígitos) ou IP[:porta] (com [lan]). */
    val target: String,
    val lan: Boolean,
    val password: String = "",
    val name: String = "",
    val totpCode: String? = null,
    /** Servidor de diretório `host:porta` (para ID curto). */
    val directory: String = "",
    /** Rótulo mostrado na sessão (nome do favorito); padrão = target. */
    val label: String = "",
)

sealed interface ConnectResult {
    data class Ok(val session: ActiveSession) : ConnectResult
    data object NeedTwoFa : ConnectResult
    data class Error(val message: String) : ConnectResult
}

/** Recebe o vídeo da sessão (implementado pela tela de sessão com MediaCodec). */
interface VideoSink {
    fun onH264(width: Int, height: Int, key: Boolean, data: ByteArray)
    fun onBgra(width: Int, height: Int, bgra: ByteArray)
}

/** Uma sessão de controle ativa: comandos (via [session]) e eventos (flows). */
class ActiveSession internal constructor(
    val session: Session,
    val info: SessionInfo,
    val label: String,
    private val listener: Listener,
    private val onEnded: (ActiveSession) -> Unit,
) {
    private val _screen = MutableStateFlow(info.width.toInt() to info.height.toInt())
    /** Tamanho do monitor capturado do host (muda ao trocar de monitor/resolução). */
    val screen: StateFlow<Pair<Int, Int>> = _screen.asStateFlow()

    private val _monitors = MutableStateFlow<List<Monitor>>(emptyList())
    val monitors: StateFlow<List<Monitor>> = _monitors.asStateFlow()

    private val _chat = MutableStateFlow<List<ChatLine>>(emptyList())
    val chat: StateFlow<List<ChatLine>> = _chat.asStateFlow()

    private val _ended = MutableStateFlow(false)
    val ended: StateFlow<Boolean> = _ended.asStateFlow()

    private val _notices = MutableSharedFlow<Notice>(extraBufferCapacity = 16)
    /** Avisos pontuais: arquivo recebido, texto copiado no host… */
    val notices: SharedFlow<Notice> = _notices.asSharedFlow()

    @Volatile var audioSink: ((rate: Int, channels: Int, pcm: ByteArray) -> Unit)? = null
    @Volatile var clipboardSink: ((String) -> Unit)? = null

    /** Registra (ou remove, com null) quem desenha o vídeo. */
    fun setVideoSink(sink: VideoSink?) = listener.setSink(sink)

    fun sendChat(text: String) {
        if (text.isBlank()) return
        session.chat(text)
        _chat.value = _chat.value + ChatLine(mine = true, text = text)
    }

    fun close() = session.hangup()

    sealed interface Notice {
        data class FileReceived(val path: String) : Notice
        data object ClipboardFromHost : Notice
    }

    data class ChatLine(val mine: Boolean, val text: String)

    /** Recebe os callbacks do Rust (thread de fundo). */
    internal class Listener : SessionListener {
        private val lock = Any()
        // Eventos que chegam antes de a sessão Kotlin existir (logo após o connect).
        private var earlyMonitors: List<Monitor>? = null
        private var earlyEnd = false
        @Volatile var owner: ActiveSession? = null
            set(o) {
                synchronized(lock) {
                    field = o
                    if (o != null) {
                        earlyMonitors?.let { o._monitors.value = it }
                        if (earlyEnd) { o._ended.value = true; o.onEnded(o) }
                    }
                }
            }
        private var sink: VideoSink? = null
        // Enquanto não há sink (a tela ainda não montou o Surface), guarda o GOP atual.
        private val pending = ArrayList<Pair<Triple<Int, Int, Boolean>, ByteArray>>()

        fun setSink(s: VideoSink?) {
            synchronized(lock) {
                sink = s
                if (s != null) {
                    pending.forEach { (h, d) -> s.onH264(h.first, h.second, h.third, d) }
                    pending.clear()
                }
            }
        }

        override fun onVideoH264(width: UInt, height: UInt, key: Boolean, data: ByteArray) {
            synchronized(lock) {
                val s = sink
                if (s != null) {
                    s.onH264(width.toInt(), height.toInt(), key, data)
                } else {
                    if (key) pending.clear()
                    if (pending.isNotEmpty() || key) {
                        if (pending.size < 240) pending.add(Triple(width.toInt(), height.toInt(), key) to data)
                    }
                }
            }
        }

        override fun onVideoBgra(width: UInt, height: UInt, bgra: ByteArray) {
            synchronized(lock) { sink }?.onBgra(width.toInt(), height.toInt(), bgra)
        }

        override fun onScreenChanged(width: UInt, height: UInt) {
            owner?._screen?.value = width.toInt() to height.toInt()
        }

        override fun onMonitors(monitors: List<Monitor>) {
            synchronized(lock) {
                val o = owner
                if (o == null) earlyMonitors = monitors else o._monitors.value = monitors
            }
        }

        override fun onAudio(rate: UInt, channels: UShort, pcm: ByteArray) {
            owner?.audioSink?.invoke(rate.toInt(), channels.toInt(), pcm)
        }

        override fun onClipboard(text: String) {
            val o = owner ?: return
            o.clipboardSink?.invoke(text)
            o._notices.tryEmit(Notice.ClipboardFromHost)
        }

        override fun onChat(text: String) {
            val o = owner ?: return
            o._chat.value = o._chat.value + ChatLine(mine = false, text = text)
        }

        override fun onFileReceived(path: String) {
            owner?._notices?.tryEmit(Notice.FileReceived(path))
        }

        override fun onDisconnected() {
            val o = synchronized(lock) { owner.also { if (it == null) earlyEnd = true } } ?: return
            o._ended.value = true
            o.onEnded(o)
        }
    }
}
