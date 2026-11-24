package io.github.ricardomendes95.elo.ui.home

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.provider.Settings
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.ricardomendes95.elo.data.Prefs
import io.github.ricardomendes95.elo.host.HostService
import io.github.ricardomendes95.elo.host.HostState
import io.github.ricardomendes95.elo.ui.CopyIconButton
import io.github.ricardomendes95.elo.ui.EloCard
import io.github.ricardomendes95.elo.ui.Feedback
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.delay
import io.github.ricardomendes95.elo.ui.PasswordField
import io.github.ricardomendes95.elo.ui.theme.EloTheme

/** Cartão "Compartilhar esta tela": liga/desliga o host (MediaProjection + serviço). */
@Composable
fun ShareCard() {
    val c = EloTheme.colors
    val ctx = LocalContext.current
    val running by HostState.running.collectAsState()
    val address by HostState.address.collectAsState()
    val status by HostState.status.collectAsState()
    val peer by HostState.peer.collectAsState()
    val a11y by HostState.accessibility.collectAsState()
    val error by HostState.error.collectAsState()
    val password by Prefs.hostPassword.collectAsState()
    val lanIp = remember(running) { HostState.lanIp() }

    // Ligando/desligando: Switch desabilitado + progresso até o estado real mudar (ou 15 s).
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(running, error) { busy = false }
    LaunchedEffect(busy) { if (busy) { delay(15_000); busy = false } }
    var ending by remember { mutableStateOf(false) }
    LaunchedEffect(peer) { ending = false }

    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val data = r.data
        if (r.resultCode == Activity.RESULT_OK && data != null) {
            HostState.setError(null)
            try {
                busy = true
                HostService.start(ctx, r.resultCode, data)
                Feedback.show("Ligando o compartilhamento…")
            } catch (e: Exception) {
                busy = false
                HostState.setError("Não foi possível ligar o compartilhamento.")
                Feedback.error("Elo", "Não foi possível ligar o compartilhamento.", e)
            }
        } else {
            HostState.setError("Permissão de captura de tela negada.")
            Feedback.error("Elo", "Permissão de captura de tela negada.")
        }
    }

    EloCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Compartilhar esta tela", fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
                Text(
                    if (running) "Ligado" else "Desligado",
                    color = if (running) c.ok else c.muted, fontSize = 12.sp,
                )
            }
            if (busy) CircularProgressIndicator(Modifier.size(20.dp).padding(end = 0.dp), strokeWidth = 2.dp, color = c.accent)
            Spacer(Modifier.size(8.dp))
            Switch(
                checked = running,
                enabled = !busy,
                onCheckedChange = { on ->
                    try {
                        if (on) {
                            val mpm = ctx.getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
                            consent.launch(mpm.createScreenCaptureIntent())
                        } else {
                            busy = true
                            HostService.stop(ctx)
                            Feedback.show("Compartilhamento desligado")
                        }
                    } catch (e: Exception) {
                        busy = false
                        Feedback.error("Elo", "Não foi possível alterar o compartilhamento.", e)
                    }
                },
                colors = SwitchDefaults.colors(checkedTrackColor = c.accent, checkedThumbColor = c.onAccent),
            )
        }
        Text(
            "Deixa um computador com o Elo ver e controlar este celular. O Android pede a sua permissão " +
                "a cada vez que você liga.",
            color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp),
        )

        error?.let { Text(it, color = c.danger, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp)) }

        Spacer(Modifier.size(10.dp))
        if (!running) {
            PasswordField(password, { Prefs.setHostPassword(it) }, "Senha (opcional)")
            Text(
                "Sem senha, cada pedido de acesso aparece aqui para você aceitar ou recusar.",
                color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 4.dp),
            )
        } else {
            if (status.isNotBlank()) Text(status, fontSize = 14.sp)
            Spacer(Modifier.size(8.dp))
            InfoLine("Endereço Elo (P2P)", address?.let { if (it.length > 30) it.take(14) + "…" + it.takeLast(14) else it } ?: "iniciando…", "Endereço Elo", "Endereço copiado") {
                address
            }
            if (lanIp != null) InfoLine("Rede local", "$lanIp:9000", "IP", "IP copiado") { lanIp }
            Text(
                if (password.isBlank()) "Acesso: com a sua aprovação." else "Acesso: com a senha definida.",
                color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 4.dp),
            )
            if (peer != null) {
                OutlinedButton(
                    enabled = !ending,
                    onClick = {
                        ending = true
                        HostState.endSession()
                        Feedback.show("Encerrando a sessão…")
                    }, shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.padding(top = 10.dp),
                ) {
                    if (ending) { CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp); Spacer(Modifier.size(8.dp)) }
                    Text("Encerrar sessão com $peer")
                }
            }
        }

        if (!a11y) {
            Spacer(Modifier.size(12.dp))
            Text(
                "Controle remoto desligado: sem o serviço de acessibilidade do Elo, o computador só vê a tela.",
                color = c.warn, fontSize = 13.sp,
            )
            Text(
                "Em Android 13+, se a opção aparecer apagada, abra Informações do app > menu ⋮ > " +
                    "\"Permitir configurações restritas\" e tente de novo.",
                color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 2.dp),
            )
            OutlinedButton(
                onClick = {
                    try {
                        ctx.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    } catch (e: Exception) {
                        Feedback.error("Elo", "Não foi possível abrir as configurações de acessibilidade.", e)
                    }
                },
                shape = RoundedCornerShape(12.dp), modifier = Modifier.padding(top = 8.dp),
            ) { Text("Abrir configurações de acessibilidade") }
        } else {
            Text("Controle remoto ligado (acessibilidade).", color = c.ok, fontSize = 12.sp, modifier = Modifier.padding(top = 10.dp))
        }
    }
}

@Composable
private fun InfoLine(label: String, value: String, clipLabel: String, okMessage: String, text: () -> String?) {
    val c = EloTheme.colors
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Column(Modifier.weight(1f)) {
            Text(label, color = c.muted, fontSize = 12.sp)
            Text(value, fontSize = 14.sp, maxLines = 1)
        }
        CopyIconButton(text(), clipLabel, okMessage, "Copiar $label")
    }
}

/** Diálogo de pedido de acesso ao host (mostrado em qualquer aba). */
@Composable
fun HostApprovalDialog() {
    val req by HostState.approval.collectAsState()
    val r = req ?: return
    AlertDialog(
        onDismissRequest = {},
        title = { Text("Pedido de acesso") },
        text = {
            Column {
                Text("\"${r.name}\" quer ver e controlar este celular.")
                Text(
                    "Impressão digital: ${r.fingerprint.take(16)}…",
                    color = EloTheme.colors.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp),
                )
            }
        },
        confirmButton = { TextButton(onClick = { HostState.respondApproval(true); Feedback.show("Acesso aceito") }) { Text("Aceitar") } },
        dismissButton = { TextButton(onClick = { HostState.respondApproval(false); Feedback.show("Acesso recusado") }) { Text("Recusar") } },
    )
}
