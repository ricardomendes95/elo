package io.github.ricardomendes95.elo.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.util.Log
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.graphics.Color
import io.github.ricardomendes95.elo.ui.theme.EloTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Retorno visual único do app: Snackbar global (hospedado no Scaffold de [EloApp]). */
object Feedback {
    val host = SnackbarHostState()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    fun show(message: String) {
        scope.launch {
            host.currentSnackbarData?.dismiss()
            host.showSnackbar(message, duration = SnackbarDuration.Short)
        }
    }

    /** Mostra e registra um erro de ação do usuário. */
    fun error(tag: String, message: String, t: Throwable? = null) {
        Log.w(tag, message, t)
        scope.launch {
            host.currentSnackbarData?.dismiss()
            host.showSnackbar(message, duration = SnackbarDuration.Long)
        }
    }
}

/** Copia [text] para a área de transferência; devolve se deu certo (erros viram aviso). */
fun copyToClipboard(ctx: Context, label: String, text: String, okMessage: String): Boolean =
    try {
        val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText(label, text))
        Feedback.show(okMessage)
        true
    } catch (e: Exception) {
        Feedback.error("Elo", "Não foi possível copiar.", e)
        false
    }

/** Botão de ícone "copiar": vira um Check por ~1,5 s e mostra o aviso. */
@Composable
fun CopyIconButton(
    text: String?,
    label: String,
    okMessage: String,
    description: String,
    tint: Color = EloTheme.colors.accent,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var done by remember { mutableStateOf(false) }
    IconButton(enabled = text != null, onClick = {
        if (text != null && copyToClipboard(ctx, label, text, okMessage)) {
            scope.launch { done = true; delay(1500); done = false }
        }
    }) {
        Icon(
            if (done) Icons.Filled.Check else Icons.Filled.ContentCopy,
            contentDescription = if (done) okMessage else description,
            tint = if (done) EloTheme.colors.ok else tint,
        )
    }
}
