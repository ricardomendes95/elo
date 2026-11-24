package io.github.ricardomendes95.elo.ui.session

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.SurfaceTexture
import android.net.Uri
import android.provider.OpenableColumns
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Chat
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.KeyboardCommandKey
import androidx.compose.material.icons.filled.Monitor
import androidx.compose.material.icons.filled.Mouse
import androidx.compose.material.icons.filled.PowerSettingsNew
import androidx.compose.material.icons.filled.TouchApp
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material.icons.filled.UploadFile
import androidx.compose.material.icons.filled.VolumeOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import io.github.ricardomendes95.elo.core.ActiveSession
import io.github.ricardomendes95.elo.data.Prefs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.roundToInt

private val Teal = Color(0xFF2DD4BF)
private val Indigo = Color(0xFF8B97F8)
private val BarBg = Color(0xE6121821)
private val BarBorder = Color(0xFF2C3745)
private val TextMain = Color(0xFFE7ECF2)
private val TextMuted = Color(0xFF98A4B3)
private val Danger = Color(0xFFF26D72)

/** Maior arquivo que o app envia de uma vez (o envio carrega o arquivo inteiro na memória). */
private const val MAX_SEND_BYTES = 64L * 1024 * 1024

/**
 * Tela de sessão: vídeo do host em tela cheia + controle por toque e teclado.
 *
 * Gestos (ver [sessionGestures]): toque = clique, dois dedos tocando ou pressionar =
 * clique direito, arrastar = arrastar (Direto) / mover o cursor (Touchpad), dois
 * dedos deslizando = rolagem, pinça = zoom local. A alça no topo abre a barra de
 * ações (teclado, teclas especiais, modo de toque, monitor, Ctrl+Alt+Del, som,
 * área de transferência, chat, enviar arquivo, informações, sair).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionScreen(active: ActiveSession, onExit: () -> Unit) {
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val session = active.session
    val info = active.info
    val canControl = info.control

    val screen by active.screen.collectAsState()
    val monitors by active.monitors.collectAsState()
    val chat by active.chat.collectAsState()
    val ended by active.ended.collectAsState()

    var touchpad by remember { mutableStateOf(Prefs.touchMode.value == "touchpad") }
    var audioOn by remember { mutableStateOf(Prefs.audioOnStart.value && info.audio) }
    var barOpen by remember { mutableStateOf(true) }
    var specialKeys by remember { mutableStateOf(false) }
    var keyboardOpen by remember { mutableStateOf(false) }
    var chatOpen by remember { mutableStateOf(false) }
    var infoOpen by remember { mutableStateOf(false) }
    var confirmExit by remember { mutableStateOf(false) }
    var monitorMenu by remember { mutableStateOf(false) }
    var actionsMenu by remember { mutableStateOf(false) }
    var seenChat by remember { mutableIntStateOf(0) }
    var fps by remember { mutableIntStateOf(0) }
    var mods by remember { mutableStateOf<Map<Int, Boolean>>(emptyMap()) }

    // Viewport (zoom/pan locais) e cursor do modo Touchpad (em px do host).
    var zoom by remember { mutableFloatStateOf(1f) }
    var pan by remember { mutableStateOf(Offset.Zero) }
    var cursor by remember { mutableStateOf(Offset(screen.first / 2f, screen.second / 2f)) }

    val keyboard = remember(session) { RemoteKeyboard(session) { canControl } }
    DisposableEffect(keyboard) {
        keyboard.onModsChanged = { mods = it }
        onDispose { keyboard.onModsChanged = null }
    }
    var captureView by remember { mutableStateOf<KeyCaptureView?>(null) }

    // ---- áudio, área de transferência, avisos ----
    val audio = remember { AudioOut() }
    DisposableEffect(active) {
        session.setAudio(audioOn)
        active.audioSink = { rate, ch, pcm -> audio.push(rate, ch, pcm) }
        active.clipboardSink = { text ->
            // Gravar no ClipboardManager tem de acontecer na thread principal.
            android.os.Handler(context.mainLooper).post {
                runCatching {
                    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    cm.setPrimaryClip(ClipData.newPlainText("Elo", text))
                }
            }
        }
        onDispose {
            active.audioSink = null
            active.clipboardSink = null
            audio.release()
        }
    }
    LaunchedEffect(active) {
        active.notices.collect { n ->
            val msg = when (n) {
                is ActiveSession.Notice.FileReceived -> "Arquivo recebido: ${n.path.substringAfterLast('/')}"
                ActiveSession.Notice.ClipboardFromHost -> "Copiado para a área de transferência."
            }
            Toast.makeText(context, msg, Toast.LENGTH_SHORT).show()
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            kotlinx.coroutines.delay(1000)
            fps = frameCounter.getAndSet(0)
        }
    }

    // ---- tela cheia imersiva + manter a tela ligada ----
    val activity = context as? Activity
    DisposableEffect(activity) {
        val window = activity?.window
        val ctl = window?.let { WindowCompat.getInsetsController(it, it.decorView) }
        ctl?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        ctl?.hide(WindowInsetsCompat.Type.systemBars())
        if (Prefs.keepScreenOn.value) window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose {
            ctl?.show(WindowInsetsCompat.Type.systemBars())
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    BackHandler { if (keyboardOpen) { captureView?.hideKeyboard(); keyboardOpen = false } else confirmExit = true }

    // ---- enviar arquivo ----
    val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching {
                    val cr = context.contentResolver
                    var name = "arquivo"
                    var size = -1L
                    cr.query(uri, null, null, null, null)?.use { c ->
                        if (c.moveToFirst()) {
                            val ni = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                            val si = c.getColumnIndex(OpenableColumns.SIZE)
                            if (ni >= 0) name = c.getString(ni) ?: name
                            if (si >= 0 && !c.isNull(si)) size = c.getLong(si)
                        }
                    }
                    if (size > MAX_SEND_BYTES) error("Arquivo grande demais (máx. 64 MB).")
                    val bytes = cr.openInputStream(uri)?.use { it.readBytes() } ?: error("Não foi possível ler o arquivo.")
                    if (bytes.size > MAX_SEND_BYTES) error("Arquivo grande demais (máx. 64 MB).")
                    session.sendFile(name, bytes)
                    name
                }
            }
            Toast.makeText(
                context,
                result.fold({ "Enviando \"$it\" para a pasta de downloads do computador." }, { it.message ?: "Falha ao enviar." }),
                Toast.LENGTH_LONG,
            ).show()
        }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        // ---- vídeo + gestos ----
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val density = LocalDensity.current
            val cw = constraints.maxWidth.toFloat()
            val ch = constraints.maxHeight.toFloat()
            val (sw, sh) = screen
            val aspect = if (sw > 0 && sh > 0) sw.toFloat() / sh else 16f / 9f
            val baseW: Float
            val baseH: Float
            if (cw / ch > aspect) { baseH = ch; baseW = ch * aspect } else { baseW = cw; baseH = cw / aspect }
            val baseLeft = (cw - baseW) / 2f
            val baseTop = (ch - baseH) / 2f

            fun clampPan(p: Offset, z: Float): Offset {
                // Mantém a imagem cobrindo a área base (sem bordas pretas ao dar zoom).
                val minX = baseW * (1 - z)
                val minY = baseH * (1 - z)
                return Offset(p.x.coerceIn(minX, 0f), p.y.coerceIn(minY, 0f))
            }

            /** Ponto da tela → pixel do monitor capturado do host. */
            fun toHost(p: Offset): Offset {
                val lx = (p.x - baseLeft - pan.x) / zoom
                val ly = (p.y - baseTop - pan.y) / zoom
                val x = (lx / baseW * (sw - 1)).coerceIn(0f, (sw - 1).toFloat())
                val y = (ly / baseH * (sh - 1)).coerceIn(0f, (sh - 1).toFloat())
                return Offset(x, y)
            }

            /** Pixel do host → ponto da tela. */
            fun toScreen(h: Offset): Offset =
                Offset(baseLeft + pan.x + h.x / (sw - 1).coerceAtLeast(1) * baseW * zoom, baseTop + pan.y + h.y / (sh - 1).coerceAtLeast(1) * baseH * zoom)

            fun move(h: Offset) {
                val x = h.x.roundToInt()
                val y = h.y.roundToInt()
                if (canControl) session.mouseMove(x, y) else session.pointer(x, y)
            }

            fun click(button: UByte, at: Offset) {
                move(at)
                if (!canControl) return
                session.mouseButton(button, true)
                session.mouseButton(button, false)
            }

            /** No Touchpad com zoom, arrasta a imagem para o cursor continuar visível. */
            fun followCursor(h: Offset) {
                if (zoom <= 1f) return
                val s = toScreen(h)
                val margin = 48f * density.density
                var p = pan
                if (s.x < baseLeft + margin) p += Offset(baseLeft + margin - s.x, 0f)
                if (s.x > baseLeft + baseW - margin) p -= Offset(s.x - (baseLeft + baseW - margin), 0f)
                if (s.y < baseTop + margin) p += Offset(0f, baseTop + margin - s.y)
                if (s.y > baseTop + baseH - margin) p -= Offset(0f, s.y - (baseTop + baseH - margin))
                pan = clampPan(p, zoom)
            }

            val target = remember(touchpad, screen, baseW, baseH, canControl) {
                object : GestureTarget {
                    override fun tap(at: Offset) {
                        click(0u, if (touchpad) cursor else toHost(at).also { cursor = it })
                    }

                    override fun twoFingerTap(at: Offset) {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        click(1u, if (touchpad) cursor else toHost(at).also { cursor = it })
                    }

                    override fun longPress(at: Offset) {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        click(1u, if (touchpad) cursor else toHost(at).also { cursor = it })
                    }

                    override fun dragStart(at: Offset, afterTap: Boolean) {
                        if (touchpad) {
                            // Toque + pressionar-arrastar = arrastar com o botão esquerdo.
                            if (afterTap && canControl) session.mouseButton(0u, true)
                            dragHeld = afterTap && canControl
                        } else {
                            val h = toHost(at)
                            cursor = h
                            move(h)
                            if (canControl) session.mouseButton(0u, true)
                            dragHeld = canControl
                        }
                    }

                    override fun drag(at: Offset, delta: Offset) {
                        if (touchpad) {
                            // Movimento relativo: 1 px do dedo ≈ 1,4 px da imagem na tela.
                            val k = 1.4f / (baseW * zoom) * sw
                            val h = Offset(
                                (cursor.x + delta.x * k).coerceIn(0f, (sw - 1).toFloat()),
                                (cursor.y + delta.y * k).coerceIn(0f, (sh - 1).toFloat()),
                            )
                            cursor = h
                            move(h)
                            followCursor(h)
                        } else {
                            val h = toHost(at)
                            cursor = h
                            move(h)
                        }
                    }

                    override fun dragEnd() {
                        if (dragHeld) session.mouseButton(0u, false)
                        dragHeld = false
                    }

                    override fun scroll(notchesX: Int, notchesY: Int) {
                        if (!canControl) return
                        if (!touchpad) move(cursor)
                        session.scroll(notchesX, notchesY)
                    }

                    override fun zoom(factor: Float, centroid: Offset, panDelta: Offset) {
                        val old = zoom
                        val z = (old * factor).coerceIn(1f, 5f)
                        // Mantém o ponto sob os dedos parado enquanto amplia, e acompanha o deslocamento.
                        val local = centroid - Offset(baseLeft, baseTop)
                        val p = local - (local - pan) * (z / old) + panDelta
                        zoom = z
                        pan = clampPan(p, z)
                    }

                    private var dragHeld = false
                }
            }

            AndroidView(
                factory = { ctx -> videoView(ctx, active) },
                modifier = Modifier
                    .offset { IntOffset(baseLeft.roundToInt(), baseTop.roundToInt()) }
                    .size(with(density) { baseW.toDp() }, with(density) { baseH.toDp() })
                    .graphicsLayer {
                        transformOrigin = TransformOrigin(0f, 0f)
                        scaleX = zoom
                        scaleY = zoom
                        translationX = pan.x
                        translationY = pan.y
                    },
            )

            // Camada de toque (cobre a tela toda).
            Box(
                Modifier
                    .fillMaxSize()
                    .sessionGestures(target, LocalViewConfiguration.current),
            )

            // Cursor local no modo Touchpad.
            if (touchpad) {
                val s = toScreen(cursor)
                Canvas(Modifier.fillMaxSize()) {
                    val r = 7.dp.toPx()
                    drawCircle(Color.Black.copy(alpha = 0.55f), r + 2.dp.toPx(), s)
                    drawCircle(Teal, r, s, style = Stroke(2.5.dp.toPx()))
                    drawCircle(Teal, 2.dp.toPx(), s)
                }
            }

            // Campo invisível que recebe o teclado do Android.
            AndroidView(
                factory = { ctx -> KeyCaptureView(ctx) { keyboard }.also { captureView = it } },
                modifier = Modifier.size(1.dp),
            )
        }

        // ---- alça + barra de ações ----
        Column(
            Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 6.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Box(
                Modifier
                    .clip(RoundedCornerShape(50))
                    .background(BarBg)
                    .border(1.dp, BarBorder, RoundedCornerShape(50))
                    .clickable { barOpen = !barOpen }
                    .padding(horizontal = 14.dp, vertical = 4.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(8.dp).clip(CircleShape).background(if (ended) Danger else Teal))
                    Spacer(Modifier.width(8.dp))
                    Text(active.label, color = TextMain, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 180.dp))
                    if (!canControl) {
                        Spacer(Modifier.width(8.dp))
                        Text("só visualização", color = Indigo, fontSize = 11.sp)
                    }
                }
            }
            AnimatedVisibility(barOpen, enter = fadeIn(), exit = fadeOut()) {
                Row(
                    Modifier
                        .padding(top = 6.dp)
                        .clip(RoundedCornerShape(14.dp))
                        .background(BarBg)
                        .border(1.dp, BarBorder, RoundedCornerShape(14.dp))
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    BarButton(Icons.Filled.Keyboard, "Teclado", keyboardOpen, enabled = canControl) {
                        val v = captureView ?: return@BarButton
                        if (keyboardOpen) v.hideKeyboard() else v.showKeyboard()
                        keyboardOpen = !keyboardOpen
                    }
                    BarButton(Icons.Filled.KeyboardCommandKey, "Teclas especiais", specialKeys, enabled = canControl) { specialKeys = !specialKeys }
                    BarButton(if (touchpad) Icons.Filled.Mouse else Icons.Filled.TouchApp, if (touchpad) "Modo touchpad" else "Modo direto", false) {
                        touchpad = !touchpad
                        Toast.makeText(context, if (touchpad) "Touchpad: arraste para mover o cursor; toque para clicar." else "Direto: toque onde quer clicar.", Toast.LENGTH_SHORT).show()
                    }
                    if (monitors.size > 1) {
                        Box {
                            BarButton(Icons.Filled.Monitor, "Monitor", false) { monitorMenu = true }
                            DropdownMenu(monitorMenu, onDismissRequest = { monitorMenu = false }) {
                                monitors.forEach { m ->
                                    DropdownMenuItem(
                                        text = { Text("${m.index + 1u}. ${m.name.ifBlank { "Monitor" }} — ${m.width}×${m.height}" + if (m.current) "  ✓" else "") },
                                        onClick = { monitorMenu = false; session.selectMonitor(m.index) },
                                    )
                                }
                            }
                        }
                    }
                    Box {
                        BarButton(Icons.Filled.PowerSettingsNew, "Comandos", actionsMenu, enabled = canControl) { actionsMenu = true }
                        DropdownMenu(actionsMenu, onDismissRequest = { actionsMenu = false }) {
                            DropdownMenuItem(
                                text = { Text("Ctrl+Alt+Del") },
                                onClick = {
                                    actionsMenu = false
                                    session.sendSas()
                                    Toast.makeText(context, "Ctrl+Alt+Del enviado (requer o serviço do Elo no Windows).", Toast.LENGTH_SHORT).show()
                                },
                            )
                            listOf(
                                Triple("Gerenciador de Tarefas", "Ctrl+Shift+Esc", intArrayOf(Vk.CONTROL, Vk.SHIFT, Vk.ESCAPE)),
                                Triple("Executar", "Win+R", intArrayOf(Vk.LWIN, 'R'.code)),
                                Triple("Histórico da área de transferência", "Win+V", intArrayOf(Vk.LWIN, 'V'.code)),
                                Triple("Captura de tela", "Win+Shift+S", intArrayOf(Vk.LWIN, Vk.SHIFT, 'S'.code)),
                            ).forEach { (label, keys, vks) ->
                                DropdownMenuItem(
                                    text = { Text("$keys  ·  $label") },
                                    onClick = {
                                        actionsMenu = false
                                        keyboard.combo(*vks)
                                    },
                                )
                            }
                        }
                    }
                    BarButton(if (audioOn) Icons.Filled.VolumeUp else Icons.Filled.VolumeOff, "Som", audioOn, enabled = info.audio) {
                        audioOn = !audioOn
                        session.setAudio(audioOn)
                        if (!audioOn) audio.release()
                    }
                    BarButton(Icons.Filled.Upload, "Enviar área de transferência ao PC", false) {
                        if (!info.clipboard) {
                            Toast.makeText(context, "O computador não liberou a área de transferência nesta sessão.", Toast.LENGTH_LONG).show()
                            return@BarButton
                        }
                        // No Android 10+ a leitura do clipboard exige o app em foco: tratamos null/vazio.
                        val text = runCatching {
                            val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
                        }.getOrNull()
                        if (text.isNullOrEmpty()) {
                            Toast.makeText(context, "Não há texto na área de transferência deste aparelho (copie algo e tente de novo).", Toast.LENGTH_LONG).show()
                        } else {
                            session.sendClipboard(text)
                            Toast.makeText(context, "Área de transferência enviada ao computador. Cole com Ctrl+V.", Toast.LENGTH_SHORT).show()
                        }
                    }
                    val unread = chat.size - seenChat
                    BadgedBox(badge = { if (unread > 0 && !chatOpen) Badge { Text("$unread") } }) {
                        BarButton(Icons.Filled.Chat, "Chat", chatOpen) { chatOpen = true }
                    }
                    BarButton(Icons.Filled.UploadFile, "Enviar arquivo", false, enabled = info.fileTransfer) {
                        pickFile.launch(arrayOf("*/*"))
                    }
                    BarButton(Icons.Filled.Info, "Informações", infoOpen) { infoOpen = true }
                    BarButton(Icons.Filled.Close, "Desconectar", false, tint = Danger) { confirmExit = true }
                }
            }
        }

        // ---- teclas especiais (acima do teclado) ----
        if (specialKeys && canControl) {
            SpecialKeysBar(
                keyboard = keyboard,
                mods = mods,
                modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding(),
            )
        }

        // ---- indicadores ----
        Text(
            "${screen.first}×${screen.second} · $fps qps" + if (zoom > 1.01f) " · ${(zoom * 100).roundToInt()}%" else "",
            color = TextMuted,
            fontSize = 10.sp,
            fontFamily = FontFamily.Monospace,
            modifier = Modifier.align(Alignment.BottomStart).navigationBarsPadding().padding(6.dp),
        )
    }

    // ---- chat ----
    if (chatOpen) {
        LaunchedEffect(chat.size) { seenChat = chat.size }
        ModalBottomSheet(onDismissRequest = { chatOpen = false; seenChat = chat.size }) {
            ChatSheet(active)
        }
    }

    // ---- informações ----
    if (infoOpen) {
        AlertDialog(
            onDismissRequest = { infoOpen = false },
            confirmButton = { TextButton(onClick = { infoOpen = false }) { Text("Fechar") } },
            title = { Text("Sessão") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("Conectado a: ${active.label}")
                    Text("Tela: ${screen.first}×${screen.second}")
                    Text(
                        "Permissões: " + listOfNotNull(
                            if (info.control) "controle" else "só visualização",
                            "arquivos".takeIf { info.fileTransfer },
                            "área de transferência".takeIf { info.clipboard },
                            "som".takeIf { info.audio },
                        ).joinToString(", "),
                    )
                    Text("Impressão digital do computador:", color = TextMuted, fontSize = 12.sp)
                    Text(info.fingerprint.chunked(4).take(8).joinToString(" "), fontFamily = FontFamily.Monospace, fontSize = 13.sp)
                    Text(
                        "Gestos: toque = clique · dois dedos tocando ou pressionar = clique direito · " +
                            "arrastar = arrastar (Direto) ou mover o cursor (Touchpad) · tocar e logo arrastar = arrastar (Touchpad) · " +
                            "dois dedos deslizando = rolar · pinça = zoom.",
                        color = TextMuted,
                        fontSize = 12.sp,
                    )
                }
            },
        )
    }

    // ---- sair / fim ----
    if (confirmExit && !ended) {
        AlertDialog(
            onDismissRequest = { confirmExit = false },
            confirmButton = { TextButton(onClick = { confirmExit = false; onExit() }) { Text("Desconectar", color = Danger) } },
            dismissButton = { TextButton(onClick = { confirmExit = false }) { Text("Continuar") } },
            title = { Text("Encerrar a sessão?") },
            text = { Text("Você vai se desconectar de ${active.label}.") },
        )
    }
    if (ended) {
        AlertDialog(
            onDismissRequest = onExit,
            confirmButton = { TextButton(onClick = onExit) { Text("OK") } },
            title = { Text("Sessão encerrada") },
            text = { Text("A conexão com ${active.label} terminou.") },
        )
    }
}

/** Contador de quadros exibidos (lido 1×/s para o indicador de qps). */
private val frameCounter = java.util.concurrent.atomic.AtomicInteger(0)

/** `TextureView` cujo Surface recebe o vídeo; liga/desliga o decoder com o Surface. */
private fun videoView(ctx: Context, active: ActiveSession): TextureView {
    val view = TextureView(ctx)
    view.isOpaque = true
    view.surfaceTextureListener = object : TextureView.SurfaceTextureListener {
        var decoder: VideoDecoder? = null
        var surface: Surface? = null

        override fun onSurfaceTextureAvailable(st: SurfaceTexture, width: Int, height: Int) {
            val s = Surface(st)
            surface = s
            val d = VideoDecoder(s)
            d.onFrame = { frameCounter.incrementAndGet() }
            decoder = d
            active.setVideoSink(d)
        }

        override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, width: Int, height: Int) = Unit

        override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
            active.setVideoSink(null)
            decoder?.release()
            decoder = null
            surface?.release()
            surface = null
            return true
        }

        override fun onSurfaceTextureUpdated(st: SurfaceTexture) = Unit
    }
    return view
}

@Composable
private fun BarButton(
    icon: ImageVector,
    label: String,
    selected: Boolean,
    enabled: Boolean = true,
    tint: Color = TextMain,
    onClick: () -> Unit,
) {
    IconButton(onClick = onClick, enabled = enabled) {
        Box(
            Modifier
                .size(36.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(if (selected) Teal.copy(alpha = 0.18f) else Color.Transparent),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, contentDescription = label, tint = if (!enabled) TextMuted.copy(alpha = 0.4f) else if (selected) Teal else tint)
        }
    }
}

/** Barra de teclas especiais: modificadores (toque = uma vez, 2º toque = travado), Esc, Tab, setas, F1–F12… */
@Composable
private fun SpecialKeysBar(keyboard: RemoteKeyboard, mods: Map<Int, Boolean>, modifier: Modifier) {
    val keys: List<Pair<String, () -> Unit>> = listOf(
        "Esc" to { keyboard.tap(Vk.ESCAPE) },
        "Tab" to { keyboard.tap(Vk.TAB) },
        "←" to { keyboard.tap(Vk.LEFT) },
        "↑" to { keyboard.tap(Vk.UP) },
        "↓" to { keyboard.tap(Vk.DOWN) },
        "→" to { keyboard.tap(Vk.RIGHT) },
        "Home" to { keyboard.tap(Vk.HOME) },
        "End" to { keyboard.tap(Vk.END) },
        "PgUp" to { keyboard.tap(Vk.PRIOR) },
        "PgDn" to { keyboard.tap(Vk.NEXT) },
        "Del" to { keyboard.tap(Vk.DELETE) },
        "Ins" to { keyboard.tap(Vk.INSERT) },
        "⌫" to { keyboard.tap(Vk.BACK) },
        "Enter" to { keyboard.tap(Vk.RETURN) },
        "PrtSc" to { keyboard.tap(Vk.SNAPSHOT) },
        "Ctrl+Shift+Esc" to { keyboard.combo(Vk.CONTROL, Vk.SHIFT, Vk.ESCAPE) },
        "Alt+Tab" to { keyboard.combo(Vk.MENU, Vk.TAB) },
        "Win+D" to { keyboard.combo(Vk.LWIN, 'D'.code) },
    ) + (1..12).map { n -> "F$n" to { keyboard.tap(Vk.f(n)) } }

    Surface(
        modifier = modifier.fillMaxWidth(),
        color = BarBg,
    ) {
        Column(Modifier.padding(vertical = 4.dp)) {
            Row(
                Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                listOf("Ctrl" to Vk.CONTROL, "Alt" to Vk.MENU, "Shift" to Vk.SHIFT, "Win" to Vk.LWIN).forEach { (label, vk) ->
                    val state = mods[vk]
                    KeyChip(
                        label = if (state == true) "$label 🔒" else label,
                        active = state != null,
                        onClick = { keyboard.toggleModifier(vk) },
                    )
                }
                keys.forEach { (label, action) -> KeyChip(label, false, action) }
            }
        }
    }
}

@Composable
private fun KeyChip(label: String, active: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .heightIn(min = 38.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(if (active) Teal else Color(0xFF202935))
            .border(1.dp, if (active) Teal else BarBorder, RoundedCornerShape(8.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = if (active) Color(0xFF052420) else TextMain, fontSize = 14.sp, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun ChatSheet(active: ActiveSession) {
    val chat by active.chat.collectAsState()
    var input by remember { mutableStateOf("") }
    val list = rememberLazyListState()
    LaunchedEffect(chat.size) { if (chat.isNotEmpty()) list.animateScrollToItem(chat.size - 1) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 16.dp).imePadding()) {
        Text("Chat com ${active.label}", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        LazyColumn(Modifier.fillMaxWidth().heightIn(min = 120.dp, max = 320.dp), state = list, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (chat.isEmpty()) {
                item { Text("Nenhuma mensagem ainda.", color = TextMuted) }
            }
            items(chat) { line ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = if (line.mine) Arrangement.End else Arrangement.Start) {
                    Text(
                        line.text,
                        color = if (line.mine) Color(0xFF052420) else TextMain,
                        modifier = Modifier
                            .clip(RoundedCornerShape(12.dp))
                            .background(if (line.mine) Teal else Color(0xFF202935))
                            .padding(horizontal = 12.dp, vertical = 8.dp)
                            .widthIn(max = 280.dp),
                    )
                }
            }
        }
        Spacer(Modifier.height(8.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(
                value = input,
                onValueChange = { input = it },
                modifier = Modifier.weight(1f),
                placeholder = { Text("Mensagem") },
                singleLine = true,
            )
            IconButton(onClick = { active.sendChat(input.trim()); input = "" }, enabled = input.isNotBlank()) {
                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Enviar", tint = Teal)
            }
        }
    }
}
