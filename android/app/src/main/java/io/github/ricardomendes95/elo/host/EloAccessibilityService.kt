package io.github.ricardomendes95.elo.host

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.accessibilityservice.GestureDescription.StrokeDescription
import android.graphics.Path
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import io.github.ricardomendes95.elo.ffi.HostInput
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min

/**
 * Converte o input remoto (coordenadas do quadro capturado) em gestos e ações de
 * acessibilidade na tela real. Sem este serviço ligado a sessão é só visualização.
 *
 * - clique = toque; botão esquerdo pressionado + movimento = arrasto (traço contínuo,
 *   `willContinue`); botão direito = toque longo; roda = deslizar;
 * - Esc / Backspace com campo vazio = Voltar; Home = Início;
 * - caractere = `ACTION_SET_TEXT` no campo focado (acrescenta ao texto); Backspace
 *   remove o último caractere; Enter = `ACTION_IME_ENTER` (API 30+).
 */
class EloAccessibilityService : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())

    // Estado do ponteiro remoto (coordenadas do quadro).
    private var curX = 0f
    private var curY = 0f
    private var leftDown = false
    private var downX = 0f
    private var downY = 0f
    private var dragging = false

    // Arrasto em andamento (traço encadeado).
    private var stroke: StrokeDescription? = null
    private var strokeX = 0f   // onde o traço enviado termina (tela real)
    private var strokeY = 0f
    private var dragBusy = false
    private var releasePending = false

    // Digitação: último texto que escrevemos (o nó pode demorar a refletir).
    private var lastSet: String? = null
    private var lastSetAt = 0L

    override fun onServiceConnected() {
        instance = this
        HostState.setAccessibility(true)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        if (instance === this) instance = null
        HostState.setAccessibility(false)
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        HostState.setAccessibility(false)
        super.onDestroy()
    }

    // ---- coordenadas ----

    private fun scaleX(x: Float): Float {
        val (rw, _) = HostState.realScreenSize(this)
        val fw = HostState.frameSize.first
        return (if (fw > 0) x * rw / fw else x).coerceIn(0f, (rw - 1).toFloat())
    }

    private fun scaleY(y: Float): Float {
        val (_, rh) = HostState.realScreenSize(this)
        val fh = HostState.frameSize.second
        return (if (fh > 0) y * rh / fh else y).coerceIn(0f, (rh - 1).toFloat())
    }

    // ---- entrada ----

    private fun handle(ev: HostInput) {
        when (ev) {
            is HostInput.MouseMove -> onMove(ev.x.toFloat(), ev.y.toFloat())
            is HostInput.MouseButton -> onButton(ev.button.toInt(), ev.down)
            is HostInput.Scroll -> onScroll(ev.dx, ev.dy)
            is HostInput.Key -> if (ev.down) onKey(ev.code.toInt())
            is HostInput.Char -> onChar(ev.code.toInt())
        }
    }

    private fun onMove(x: Float, y: Float) {
        curX = x; curY = y
        if (!leftDown) return
        val slop = 12f * (HostState.frameSize.first.coerceAtLeast(1) / 720f).coerceAtLeast(1f)
        if (!dragging && hypot(x - downX, y - downY) > slop) dragging = true
        if (dragging) pumpDrag()
    }

    private fun onButton(button: Int, down: Boolean) {
        when (button) {
            0 -> if (down) {
                leftDown = true; dragging = false
                downX = curX; downY = curY
            } else if (leftDown) {
                leftDown = false
                if (dragging) {
                    releasePending = true
                    pumpDrag()
                } else {
                    tap(scaleX(downX), scaleY(downY), 50)
                }
                dragging = false
            }
            1 -> if (down) tap(scaleX(curX), scaleY(curY), 650) // toque longo
        }
    }

    private fun tap(x: Float, y: Float, durationMs: Long) {
        val p = Path().apply { moveTo(x, y) }
        dispatch(StrokeDescription(p, 0, durationMs))
    }

    private fun dispatch(s: StrokeDescription, done: (() -> Unit)? = null) {
        val g = GestureDescription.Builder().addStroke(s).build()
        val ok = dispatchGesture(g, object : GestureResultCallback() {
            override fun onCompleted(gestureDescription: GestureDescription?) { done?.invoke() }
            override fun onCancelled(gestureDescription: GestureDescription?) { done?.invoke() }
        }, main)
        if (!ok) done?.invoke()
    }

    /** Arrasto: envia o próximo trecho do traço até a posição mais recente (um por vez). */
    private fun pumpDrag() {
        if (dragBusy) return
        val tx = scaleX(curX)
        val ty = scaleY(curY)
        val first = stroke == null
        val sx = if (first) scaleX(downX) else strokeX
        val sy = if (first) scaleY(downY) else strokeY
        val finish = releasePending
        if (!finish && abs(tx - sx) < 1f && abs(ty - sy) < 1f) return
        val path = Path().apply {
            moveTo(sx, sy)
            // Soltar sem mover mais: um deslocamento mínimo evita caminho de comprimento zero.
            if (abs(tx - sx) < 1f && abs(ty - sy) < 1f) lineTo(sx + 0.5f, sy) else lineTo(tx, ty)
        }
        val dur = 40L
        val s = if (first) StrokeDescription(path, 0, dur, !finish)
        else stroke!!.continueStroke(path, 0, dur, !finish)
        dragBusy = true
        dispatch(s) {
            dragBusy = false
            strokeX = tx; strokeY = ty
            stroke = if (finish) null else s
            if (finish) {
                releasePending = false
                stroke = null
            } else {
                // Há movimento novo ou soltura pendente?
                if (releasePending || abs(scaleX(curX) - strokeX) >= 1f || abs(scaleY(curY) - strokeY) >= 1f) pumpDrag()
            }
        }
    }

    private fun onScroll(dx: Int, dy: Int) {
        val (rw, rh) = HostState.realScreenSize(this)
        val cx = if (HostState.frameSize.first > 0) scaleX(curX) else rw / 2f
        val cy = if (HostState.frameSize.second > 0) scaleY(curY) else rh / 2f
        // Roda para cima = conteúdo desce = dedo desliza para baixo; roda para a direita = dedo para a esquerda.
        val vy = (dy * rh * 0.15f).coerceIn(-rh * 0.6f, rh * 0.6f)
        val vx = (-dx * rw * 0.15f).coerceIn(-rw * 0.6f, rw * 0.6f)
        val x0 = (cx - vx / 2).coerceIn(10f, rw - 10f)
        val y0 = (cy - vy / 2).coerceIn(10f, rh - 10f)
        val x1 = (x0 + vx).coerceIn(10f, rw - 10f)
        val y1 = (y0 + vy).coerceIn(10f, rh - 10f)
        if (abs(x1 - x0) < 4f && abs(y1 - y0) < 4f) return
        val p = Path().apply { moveTo(x0, y0); lineTo(x1, y1) }
        dispatch(StrokeDescription(p, 0, 250))
    }

    private fun focusedInput(): AccessibilityNodeInfo? {
        val root = rootInActiveWindow ?: return null
        val n = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return null
        return if (n.isEditable) n else null
    }

    private fun currentText(n: AccessibilityNodeInfo): String {
        n.refresh()
        val base = if (Build.VERSION.SDK_INT >= 26 && n.isShowingHintText) "" else (n.text?.toString() ?: "")
        val ls = lastSet
        // O nó pode ainda não refletir o texto que acabamos de escrever (digitação rápida).
        return if (ls != null && SystemClock.uptimeMillis() - lastSetAt < 400 && base != ls) ls else base
    }

    private fun setText(n: AccessibilityNodeInfo, text: String) {
        val args = Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text)
        }
        n.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
        lastSet = text
        lastSetAt = SystemClock.uptimeMillis()
    }

    private fun onChar(code: Int) {
        if (code < 0x20 && code != 0x0A) return
        val n = focusedInput() ?: return
        if (code == 0x0A) { enter(n); return }
        setText(n, currentText(n) + String(Character.toChars(code)))
    }

    private fun enter(n: AccessibilityNodeInfo) {
        if (Build.VERSION.SDK_INT >= 30) n.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
    }

    private fun onKey(vk: Int) {
        when (vk) {
            0x1B -> performGlobalAction(GLOBAL_ACTION_BACK) // Esc
            0x24 -> performGlobalAction(GLOBAL_ACTION_HOME) // Home
            0x08 -> { // Backspace
                val n = focusedInput()
                val t = if (n != null) currentText(n) else ""
                if (n != null && t.isNotEmpty()) {
                    val cut = t.offsetByCodePoints(t.length, -1)
                    setText(n, t.substring(0, cut))
                } else {
                    performGlobalAction(GLOBAL_ACTION_BACK)
                }
            }
            0x0D -> focusedInput()?.let { enter(it) } // Enter
        }
    }

    companion object {
        @Volatile private var instance: EloAccessibilityService? = null

        /** Chamado de qualquer thread; executa na principal. Sem o serviço ligado, ignora. */
        fun dispatch(ev: HostInput) {
            val s = instance ?: return
            s.main.post { runCatching { s.handle(ev) } }
        }
    }
}
