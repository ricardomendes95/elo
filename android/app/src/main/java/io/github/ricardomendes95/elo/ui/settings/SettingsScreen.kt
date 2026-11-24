package io.github.ricardomendes95.elo.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.ricardomendes95.elo.core.EloCore
import io.github.ricardomendes95.elo.data.Prefs
import io.github.ricardomendes95.elo.ui.EloCard
import io.github.ricardomendes95.elo.ui.EloField
import io.github.ricardomendes95.elo.ui.Feedback
import io.github.ricardomendes95.elo.ui.theme.EloTheme

@Composable
fun SettingsScreen() {
    val c = EloTheme.colors
    val deviceName by Prefs.deviceName.collectAsState()
    val directory by Prefs.directory.collectAsState()
    val theme by Prefs.theme.collectAsState()
    val touch by Prefs.touchMode.collectAsState()
    val audio by Prefs.audioOnStart.collectAsState()
    val keepOn by Prefs.keepScreenOn.collectAsState()
    var editName by remember { mutableStateOf(false) }
    var editDir by remember { mutableStateOf(false) }
    val defaultName = remember { EloCore.defaultDeviceName() }

    Column(
        Modifier.statusBarsPadding().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Configurações", fontSize = 26.sp, fontWeight = FontWeight.Bold)

        Section("Geral") {
            Item("Nome deste aparelho", deviceName.ifBlank { defaultName }, onClick = { editName = true })
            HorizontalDivider(color = c.border)
            Choice("Tema", theme, listOf("dark" to "Escuro", "light" to "Claro", "system" to "Sistema")) { Prefs.setTheme(it); Feedback.show("Tema alterado") }
        }
        Section("Conexão") {
            Item("Servidor de diretório", directory.ifBlank { "desligado (host:porta, para IDs curtos)" }, onClick = { editDir = true })
            HorizontalDivider(color = c.border)
            Item("Porta LAN padrão", "9000 (informativa)")
        }
        Section("Sessão") {
            Choice("Modo de toque padrão", touch, listOf("direct" to "Direto", "touchpad" to "Touchpad")) { Prefs.setTouchMode(it); Feedback.show("Modo de toque salvo") }
            HorizontalDivider(color = c.border)
            Toggle("Som ligado ao entrar", audio) { Prefs.setAudioOnStart(it); Feedback.show(if (it) "Som ligado ao entrar" else "Som desligado ao entrar") }
            HorizontalDivider(color = c.border)
            Toggle("Manter a tela ligada", keepOn) { Prefs.setKeepScreenOn(it); Feedback.show(if (it) "Tela ficará ligada" else "Tela pode apagar") }
        }
        Section("Sobre") {
            Item("Versão", "0.1.0")
            HorizontalDivider(color = c.border)
            Item("Elo", "Elo — acesso remoto P2P")
            HorizontalDivider(color = c.border)
            Item("Arquivos recebidos", runCatching { EloCore.downloadDir().absolutePath }.onFailure { android.util.Log.w("Elo", "downloadDir falhou", it) }.getOrDefault("indisponível"))
        }
    }

    if (editName) TextDialog("Nome deste aparelho", deviceName.ifBlank { defaultName }, { editName = false }) {
        Prefs.setDeviceName(it.trim().takeIf { n -> n != defaultName } ?: ""); editName = false
        Feedback.show("Nome salvo")
    }
    if (editDir) TextDialog("Servidor de diretório (host:porta)", directory, { editDir = false }) {
        Prefs.setDirectory(it); editDir = false
        Feedback.show("Servidor de diretório salvo")
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Text(title, color = EloTheme.colors.accent, fontWeight = FontWeight.SemiBold, fontSize = 14.sp, modifier = Modifier.padding(top = 8.dp))
    EloCard { content() }
}

@Composable
private fun Item(title: String, value: String, onClick: (() -> Unit)? = null) {
    Column(
        Modifier.fillMaxWidth().then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(vertical = 10.dp),
    ) {
        Text(title)
        Text(value, color = EloTheme.colors.muted, fontSize = 13.sp)
    }
}

@Composable
private fun Toggle(title: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    val c = EloTheme.colors
    Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f))
        Switch(
            checked, onChange,
            colors = SwitchDefaults.colors(checkedTrackColor = c.accent, checkedThumbColor = c.onAccent),
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Choice(title: String, current: String, options: List<Pair<String, String>>, onPick: (String) -> Unit) {
    val c = EloTheme.colors
    Column(Modifier.padding(vertical = 10.dp)) {
        Text(title, Modifier.padding(bottom = 8.dp))
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            options.forEachIndexed { i, (k, label) ->
                SegmentedButton(
                    selected = current == k, onClick = { onPick(k) },
                    shape = SegmentedButtonDefaults.itemShape(i, options.size),
                    colors = SegmentedButtonDefaults.colors(activeContainerColor = c.accent, activeContentColor = c.onAccent),
                ) { Text(label) }
            }
        }
    }
}

@Composable
private fun TextDialog(title: String, initial: String, onDismiss: () -> Unit, onSave: (String) -> Unit) {
    var v by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss, title = { Text(title) },
        text = { EloField(v, { v = it }, title) },
        confirmButton = { TextButton(onClick = { onSave(v) }) { Text("Salvar") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}
