package io.github.ricardomendes95.elo.ui.favorites

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
import io.github.ricardomendes95.elo.ui.ConnectDialogs
import io.github.ricardomendes95.elo.ui.Connector
import io.github.ricardomendes95.elo.ui.EloCard
import io.github.ricardomendes95.elo.ui.EloField
import io.github.ricardomendes95.elo.ui.Feedback
import io.github.ricardomendes95.elo.ui.PasswordField
import io.github.ricardomendes95.elo.ui.theme.EloTheme
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private fun key(f: Favorite) = "${if (f.lan) "l" else "i"}|${f.address}"

@Composable
fun FavoritesScreen() {
    val c = EloTheme.colors
    val favorites by Prefs.favorites.collectAsState()
    val directory by Prefs.directory.collectAsState()
    val presence = remember { mutableStateMapOf<String, Boolean>() }
    var editing by remember { mutableStateOf<Favorite?>(null) }
    var creating by remember { mutableStateOf(false) }
    val owner = LocalLifecycleOwner.current

    // Presença: ao abrir e a cada 45 s enquanto a tela está visível.
    LaunchedEffect(favorites, directory) {
        owner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                coroutineScope {
                    favorites.forEach { f ->
                        launch { presence[key(f)] = EloCore.isOnline(f.address, f.lan, directory) }
                    }
                }
                delay(45_000)
            }
        }
    }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.statusBarsPadding().padding(horizontal = 16.dp)) {
            Text("Favoritos", fontSize = 26.sp, fontWeight = FontWeight.Bold, modifier = Modifier.padding(vertical = 16.dp))
            if (favorites.isEmpty()) {
                Column(
                    Modifier.fillMaxSize().padding(32.dp),
                    horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center,
                ) {
                    Icon(Icons.Filled.Star, null, Modifier.size(48.dp), tint = c.muted)
                    Spacer(Modifier.size(12.dp))
                    Text("Nenhum favorito ainda", fontWeight = FontWeight.SemiBold)
                    Text(
                        "Toque em + para salvar um computador, ou na estrela da tela Início.",
                        color = c.muted, modifier = Modifier.padding(top = 4.dp),
                    )
                }
            } else {
                LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 88.dp)) {
                    items(favorites, key = { key(it) + it.name }) { f ->
                        FavoriteRow(
                            f, presence[key(f)],
                            onConnect = { Connector.start(f.address, f.lan, f.password, f.name) },
                            onEdit = { editing = f },
                            onRemove = { Prefs.setFavorites(Prefs.favorites.value - f); Feedback.show("Favorito removido") },
                        )
                    }
                }
            }
        }
        FloatingActionButton(
            onClick = { creating = true },
            modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
            containerColor = c.accent, contentColor = c.onAccent,
        ) { Icon(Icons.Filled.Add, contentDescription = "Novo favorito") }
    }

    if (creating) {
        FavoriteDialog(null, onDismiss = { creating = false }) {
            Prefs.setFavorites(Prefs.favorites.value + it); creating = false
            Feedback.show("Favorito salvo")
        }
    }
    editing?.let { old ->
        FavoriteDialog(old, onDismiss = { editing = null }) { nw ->
            Prefs.setFavorites(Prefs.favorites.value.map { if (it == old) nw.copy(lastNode = if (nw.address == old.address && nw.lan == old.lan) old.lastNode else "") else it }); editing = null
            Feedback.show("Favorito atualizado")
        }
    }
    ConnectDialogs(full = true)
}

@Composable
private fun FavoriteRow(f: Favorite, online: Boolean?, onConnect: () -> Unit, onEdit: () -> Unit, onRemove: () -> Unit) {
    val c = EloTheme.colors
    var menu by remember { mutableStateOf(false) }
    EloCard(Modifier.clickable(onClick = onConnect)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(if (f.lan) Icons.Filled.Wifi else Icons.Filled.Public, null, tint = c.accent2)
            Column(Modifier.weight(1f)) {
                Text(f.name, fontWeight = FontWeight.SemiBold, maxLines = 1)
                Text(f.address, color = c.muted, fontSize = 12.sp, maxLines = 1)
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 4.dp)) {
                    Box(Modifier.size(8.dp).background(if (online == true) c.ok else c.muted, CircleShape))
                    Spacer(Modifier.size(6.dp))
                    Text(
                        when (online) { null -> "verificando…"; true -> "online"; false -> "offline" },
                        color = c.muted, fontSize = 12.sp,
                    )
                }
            }
            Box {
                IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "Mais opções") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(text = { Text("Editar") }, onClick = { menu = false; onEdit() })
                    DropdownMenuItem(text = { Text("Remover", color = c.danger) }, onClick = { menu = false; onRemove() })
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun FavoriteDialog(initial: Favorite?, onDismiss: () -> Unit, onSave: (Favorite) -> Unit) {
    var name by remember { mutableStateOf(initial?.name ?: "") }
    var lan by remember { mutableStateOf(initial?.lan ?: false) }
    var address by remember { mutableStateOf(initial?.address ?: "") }
    var password by remember { mutableStateOf(initial?.password ?: "") }
    val c = EloTheme.colors
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (initial == null) "Novo favorito" else "Editar favorito") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                EloField(name, { name = it }, "Nome")
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    listOf("Internet", "Rede local").forEachIndexed { i, label ->
                        SegmentedButton(
                            selected = lan == (i == 1), onClick = { lan = i == 1 },
                            shape = SegmentedButtonDefaults.itemShape(i, 2),
                            colors = SegmentedButtonDefaults.colors(activeContainerColor = c.accent, activeContentColor = c.onAccent),
                        ) { Text(label) }
                    }
                }
                EloField(
                    address, { address = it },
                    if (lan) "IP do computador (ex.: 192.168.0.10)" else "Endereço Elo ou ID de 9 dígitos",
                    keyboardType = if (lan) KeyboardType.Uri else KeyboardType.Text,
                )
                PasswordField(password, { password = it }, "Senha (opcional)")
            }
        },
        confirmButton = {
            TextButton(
                enabled = name.isNotBlank() && address.isNotBlank(),
                onClick = { onSave(Favorite(name.trim(), address.trim(), lan, password)) },
            ) { Text("Salvar") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}
