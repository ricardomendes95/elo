package io.github.ricardomendes95.elo.ui.home

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.StarBorder
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import io.github.ricardomendes95.elo.core.EloCore
import io.github.ricardomendes95.elo.data.Favorite
import io.github.ricardomendes95.elo.data.Prefs
import io.github.ricardomendes95.elo.data.Recent
import io.github.ricardomendes95.elo.ffi.DiscoveredDevice
import io.github.ricardomendes95.elo.ui.ConnState
import io.github.ricardomendes95.elo.ui.ConnectDialogs
import io.github.ricardomendes95.elo.ui.Connector
import io.github.ricardomendes95.elo.ui.EloCard
import io.github.ricardomendes95.elo.ui.EloField
import io.github.ricardomendes95.elo.ui.PasswordField
import io.github.ricardomendes95.elo.ui.PrimaryButton
import io.github.ricardomendes95.elo.ui.theme.EloLogo
import io.github.ricardomendes95.elo.ui.theme.EloTheme
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen() {
    val c = EloTheme.colors
    val ctx = LocalContext.current
    val conn by Connector.state.collectAsState()
    val recents by Prefs.recents.collectAsState()
    val favorites by Prefs.favorites.collectAsState()
    val directory by Prefs.directory.collectAsState()
    var lan by remember { mutableStateOf(false) }
    var address by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var saveDialog by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<Recent?>(null) }
    val presence = remember { mutableStateMapOf<String, Boolean>() }
    val owner = LocalLifecycleOwner.current
    val connecting = conn is ConnState.Connecting

    fun connect(target: String = address, isLan: Boolean = lan, pass: String = password, label: String = "") {
        if (target.isBlank()) return
        Connector.start(target.trim(), isLan, pass, label)
    }

    // Presença dos recentes: ao abrir e a cada 45 s enquanto a tela está visível.
    LaunchedEffect(recents, directory) {
        owner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                coroutineScope {
                    recents.forEach { r ->
                        launch { presence[recentKey(r)] = EloCore.isOnline(r.address, r.lan, directory) }
                    }
                }
                delay(45_000)
            }
        }
    }

    Column(
        Modifier.statusBarsPadding().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            EloLogo(36.dp)
            Text("Elo", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.text)
        }

        EloCard {
            Text("Conectar a outro computador", fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
            Spacer(Modifier.size(12.dp))
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                listOf("Internet", "Rede local").forEachIndexed { i, label ->
                    SegmentedButton(
                        selected = lan == (i == 1), onClick = { lan = i == 1 },
                        shape = SegmentedButtonDefaults.itemShape(i, 2),
                        colors = SegmentedButtonDefaults.colors(
                            activeContainerColor = c.accent, activeContentColor = c.onAccent,
                        ),
                    ) { Text(label) }
                }
            }
            Spacer(Modifier.size(12.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                EloField(
                    address, { address = it },
                    if (lan) "IP do computador (ex.: 192.168.0.10)" else "Endereço Elo ou ID de 9 dígitos",
                    modifier = Modifier.weight(1f),
                    keyboardType = if (lan) KeyboardType.Uri else KeyboardType.Text,
                )
                IconButton(onClick = { saveDialog = true }, enabled = address.isNotBlank()) {
                    Icon(Icons.Filled.StarBorder, contentDescription = "Salvar como favorito", tint = c.warn)
                }
            }
            Spacer(Modifier.size(8.dp))
            PasswordField(password, { password = it }, "Senha (opcional)")
            Text(
                "Sem senha, o outro computador recebe um pedido para aprovar.",
                color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 4.dp, bottom = 12.dp),
            )
            PrimaryButton("Conectar", { connect() }, enabled = address.isNotBlank() && !connecting)
        }

        when (val s = conn) {
            is ConnState.Connecting -> EloCard {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp, color = c.accent)
                    Spacer(Modifier.size(12.dp))
                    Text(
                        "Conectando… se pediu aprovação, aguarde o outro lado aceitar.",
                        modifier = Modifier.weight(1f),
                    )
                }
                OutlinedButton(
                    onClick = { Connector.cancel() }, modifier = Modifier.padding(top = 12.dp),
                    shape = RoundedCornerShape(12.dp),
                ) { Text("Cancelar") }
            }
            is ConnState.Error -> EloCard(Modifier.clickable { Connector.dismiss() }) {
                Text(s.message, color = c.danger)
            }
            else -> {}
        }

        NetworkDevicesSection(
            enabled = !connecting,
            onConnect = { d -> connect("${d.ip}:${d.port}", true, "", d.name) },
        )

        DeviceCard()

        if (recents.isNotEmpty()) {
            Text("Recentes", fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
            EloCard {
                recents.forEachIndexed { i, r ->
                    if (i > 0) Spacer(Modifier.size(2.dp))
                    RecentRow(
                        r, presence[recentKey(r)], enabled = !connecting,
                        onConnect = {
                            if (r.password.isNotEmpty()) {
                                connect(r.address, r.lan, r.password, r.name)
                            } else {
                                // Sem senha salva: preenche o formulário para digitar a senha (salva ou dinâmica).
                                address = r.address; lan = r.lan; password = ""
                            }
                        },
                        onRename = { renaming = r },
                        onRemove = { Prefs.setRecents(Prefs.recents.value - r) },
                        onFavorite = {
                            val added = Prefs.addFavorite(r.name, r.address, r.lan, r.password)
                            Toast.makeText(
                                ctx,
                                if (added) "Adicionado aos favoritos." else "Já está nos favoritos.",
                                Toast.LENGTH_SHORT,
                            ).show()
                        },
                    )
                }
            }
        }
    }

    renaming?.let { r ->
        var name by remember(r) { mutableStateOf(r.name) }
        AlertDialog(
            onDismissRequest = { renaming = null },
            title = { Text("Renomear") },
            text = {
                Column {
                    EloField(name, { name = it }, "Nome")
                    Text(r.address, color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
                }
            },
            confirmButton = {
                TextButton(
                    enabled = name.isNotBlank(),
                    onClick = {
                        Prefs.setRecents(Prefs.recents.value.map { if (it == r) it.copy(name = name.trim()) else it })
                        renaming = null
                    },
                ) { Text("Salvar") }
            },
            dismissButton = { TextButton(onClick = { renaming = null }) { Text("Cancelar") } },
        )
    }

    if (saveDialog) {
        var name by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { saveDialog = false },
            title = { Text("Salvar como favorito") },
            text = { EloField(name, { name = it }, "Nome") },
            confirmButton = {
                TextButton(
                    enabled = name.isNotBlank(),
                    onClick = {
                        Prefs.setFavorites(Prefs.favorites.value + Favorite(name.trim(), address.trim(), lan, password))
                        saveDialog = false
                    },
                ) { Text("Salvar") }
            },
            dismissButton = { TextButton(onClick = { saveDialog = false }) { Text("Cancelar") } },
        )
    }
    ConnectDialogs(full = false)
}

private fun recentKey(r: Recent) = "${if (r.lan) "l" else "i"}|${r.address}"

@Composable
private fun RecentRow(
    r: Recent,
    online: Boolean?,
    enabled: Boolean,
    onConnect: () -> Unit,
    onRename: () -> Unit,
    onRemove: () -> Unit,
    onFavorite: () -> Unit,
) {
    val c = EloTheme.colors
    var menu by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onConnect).padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Icon(if (r.lan) Icons.Filled.Wifi else Icons.Filled.History, null, tint = c.accent2)
        Column(Modifier.weight(1f)) {
            Text(r.name, fontWeight = FontWeight.SemiBold, maxLines = 1)
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 2.dp)) {
                Box(Modifier.size(8.dp).background(if (online == true) c.ok else c.muted, CircleShape))
                Spacer(Modifier.size(6.dp))
                Text(
                    r.address + " · " + when (online) { null -> "verificando…"; true -> "online"; false -> "offline" },
                    color = c.muted, fontSize = 12.sp, maxLines = 1,
                )
            }
        }
        Box {
            IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "Mais opções") }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("Adicionar aos favoritos") }, onClick = { menu = false; onFavorite() })
                DropdownMenuItem(text = { Text("Renomear") }, onClick = { menu = false; onRename() })
                DropdownMenuItem(text = { Text("Remover", color = c.danger) }, onClick = { menu = false; onRemove() })
            }
        }
    }
}

/**
 * Descoberta de hosts Elo na LAN. Enquanto esta seção está na composição, segura um
 * `WifiManager.MulticastLock` (exigido para receber o broadcast UDP) e lê a lista a cada 2 s.
 */
@Composable
private fun NetworkDevicesSection(enabled: Boolean, onConnect: (DiscoveredDevice) -> Unit) {
    val c = EloTheme.colors
    val ctx = LocalContext.current
    val devices = remember { mutableStateListOf<DiscoveredDevice>() }
    var failed by remember { mutableStateOf(false) }
    var scanning by remember { mutableStateOf(false) }
    val discovery = remember { EloCore.newDiscovery() }
    val scope = rememberCoroutineScope()

    DisposableEffect(discovery) {
        val wifi = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val lock = wifi.createMulticastLock("elo-discovery").apply {
            setReferenceCounted(false)
            runCatching { acquire() }
        }
        failed = runCatching { discovery.start() }.isFailure
        onDispose {
            runCatching { discovery.stop() }
            runCatching { discovery.close() }
            runCatching { if (lock.isHeld) lock.release() }
            devices.clear()
        }
    }
    LaunchedEffect(discovery) {
        while (true) {
            val list = runCatching { discovery.devices() }.getOrDefault(emptyList())
            devices.clear()
            devices.addAll(list)
            delay(2000)
        }
    }

    Row(verticalAlignment = Alignment.CenterVertically) {
        Text("Dispositivos na rede", fontWeight = FontWeight.SemiBold, fontSize = 17.sp, modifier = Modifier.weight(1f))
        IconButton(
            enabled = !scanning,
            onClick = {
                scope.launch {
                    scanning = true
                    devices.clear()
                    runCatching { discovery.stop() }
                    failed = runCatching { discovery.start() }.isFailure
                    delay(2000)
                    scanning = false
                }
            },
        ) {
            if (scanning) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = c.accent)
            } else {
                Icon(Icons.Filled.Refresh, contentDescription = "Atualizar lista de dispositivos", tint = c.accent)
            }
        }
    }
    EloCard {
        if (scanning) {
            Text("Procurando computadores com o Elo na rede local…", color = c.muted, fontSize = 13.sp)
        } else if (devices.isEmpty()) {
            Text(
                if (failed) "Descoberta na rede indisponível neste aparelho."
                else "Procurando computadores com o Elo na rede local…",
                color = c.muted, fontSize = 13.sp,
            )
        } else {
            devices.forEachIndexed { i, d ->
                if (i > 0) Spacer(Modifier.size(2.dp))
                Row(
                    Modifier.fillMaxWidth().clickable(enabled = enabled) { onConnect(d) }.padding(vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Icon(Icons.Filled.Wifi, null, tint = c.accent2)
                    Column(Modifier.weight(1f)) {
                        Text(d.name.ifBlank { d.ip }, fontWeight = FontWeight.SemiBold, maxLines = 1)
                        Text("${d.ip}:${d.port}", color = c.muted, fontSize = 12.sp, maxLines = 1)
                    }
                }
            }
        }
    }
}

@Composable
private fun DeviceCard() {
    val c = EloTheme.colors
    val ctx = LocalContext.current
    val deviceName by Prefs.deviceName.collectAsState()
    val defaultName = remember { EloCore.defaultDeviceName() }
    val shownName = deviceName.ifBlank { defaultName }
    var editName by remember { mutableStateOf(false) }
    var id by remember { mutableStateOf<String?>(null) }
    var err by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        EloCore.deviceId().onSuccess { id = it }.onFailure { err = it.message }
    }
    EloCard {
        Text("Este dispositivo", fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
        Spacer(Modifier.size(10.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Nome", color = c.muted, fontSize = 12.sp)
                Text(shownName, fontWeight = FontWeight.SemiBold, fontSize = 15.sp, maxLines = 1)
            }
            IconButton(onClick = { editName = true }) {
                Icon(Icons.Filled.Edit, contentDescription = "Editar nome deste dispositivo", tint = c.accent)
            }
        }
        Text(
            "É o nome que os outros veem ao receber sua conexão.",
            color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 2.dp),
        )
        Spacer(Modifier.size(12.dp))
        val shown = id?.let { if (it.length > 30) it.take(14) + "…" + it.takeLast(14) else it }
        Text(
            shown ?: err ?: "iniciando P2P…",
            color = if (err != null && id == null) c.danger else c.text,
            maxLines = 1, fontSize = 14.sp,
        )
        Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                enabled = id != null, shape = RoundedCornerShape(12.dp),
                onClick = {
                    val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    cm.setPrimaryClip(ClipData.newPlainText("Endereço Elo", id))
                    Toast.makeText(ctx, "Endereço copiado", Toast.LENGTH_SHORT).show()
                },
            ) { Icon(Icons.Filled.ContentCopy, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Copiar") }
            OutlinedButton(
                enabled = id != null, shape = RoundedCornerShape(12.dp),
                onClick = {
                    val i = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, id)
                    ctx.startActivity(Intent.createChooser(i, "Compartilhar endereço").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                },
            ) { Icon(Icons.Filled.Share, null, Modifier.size(18.dp)); Spacer(Modifier.size(6.dp)); Text("Compartilhar") }
        }
        Text(
            "Para ser acessado por outro aparelho, use o Elo no computador; compartilhar a tela deste celular chega numa próxima versão.",
            color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 10.dp),
        )
    }

    if (editName) {
        var name by remember { mutableStateOf(deviceName) }
        AlertDialog(
            onDismissRequest = { editName = false },
            title = { Text("Nome deste dispositivo") },
            text = {
                Column {
                    EloField(name, { name = it }, "Nome")
                    Text(
                        "É o nome que os outros veem ao receber sua conexão. Deixe em branco para usar o nome do aparelho ($defaultName).",
                        color = c.muted, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp),
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    Prefs.setDeviceName(name.trim().takeIf { it != defaultName } ?: "")
                    editName = false
                }) { Text("Salvar") }
            },
            dismissButton = { TextButton(onClick = { editName = false }) { Text("Cancelar") } },
        )
    }
}
