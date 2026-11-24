package io.github.ricardomendes95.elo.ui

import android.util.Log
import io.github.ricardomendes95.elo.core.ConnectRequest
import io.github.ricardomendes95.elo.core.ConnectResult
import io.github.ricardomendes95.elo.core.EloCore
import io.github.ricardomendes95.elo.data.Prefs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

sealed interface ConnState {
    data object Idle : ConnState
    data class Connecting(val label: String) : ConnState
    data object NeedTwoFa : ConnState
    data class Error(val message: String) : ConnState
}

/** Controla a conexão em andamento, compartilhada entre Início e Favoritos. */
object Connector {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val _state = MutableStateFlow<ConnState>(ConnState.Idle)
    val state: StateFlow<ConnState> = _state.asStateFlow()

    private var generation = 0
    private var pending: ConnectRequest? = null

    fun start(target: String, lan: Boolean, password: String, label: String = "") {
        if (_state.value is ConnState.Connecting) return
        run(
            ConnectRequest(
                target = target, lan = lan, password = password,
                name = Prefs.deviceName.value.ifBlank { EloCore.defaultDeviceName() },
                directory = Prefs.directory.value, label = label,
            ),
        )
    }

    fun submitTotp(code: String) {
        val req = pending ?: return
        run(req.copy(totpCode = code))
    }

    /** Descarta o resultado da tentativa atual (a sessão, se vier, é fechada). */
    fun cancel() {
        generation++
        pending = null
        _state.value = ConnState.Idle
    }

    fun dismiss() {
        if (_state.value !is ConnState.Connecting) _state.value = ConnState.Idle
    }

    private fun run(req: ConnectRequest) {
        val gen = ++generation
        pending = req
        _state.value = ConnState.Connecting(req.label.ifBlank { req.target })
        scope.launch {
            val result = EloCore.connect(req)
            if (gen != generation) {
                if (result is ConnectResult.Ok) result.session.close()
                return@launch
            }
            when (result) {
                is ConnectResult.Ok -> {
                    Prefs.addRecent(req.label, req.target, req.lan, req.password)
                    pending = null
                    _state.value = ConnState.Idle
                }
                ConnectResult.NeedTwoFa -> _state.value = ConnState.NeedTwoFa
                is ConnectResult.Error -> { Log.w("Elo", "Falha ao conectar: ${result.message}"); pending = null; _state.value = ConnState.Error(result.message) }
            }
        }
    }
}
