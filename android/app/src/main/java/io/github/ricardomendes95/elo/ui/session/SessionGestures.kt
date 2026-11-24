package io.github.ricardomendes95.elo.ui.session

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlin.math.hypot

/** O que cada gesto faz na sessão; implementado pela tela. Posições em px da área de toque. */
interface GestureTarget {
    /** Toque simples. */
    fun tap(at: Offset)
    /** Toque com dois dedos. */
    fun twoFingerTap(at: Offset)
    /** Pressionar sem mover (~0,5 s). */
    fun longPress(at: Offset)
    /** Um dedo arrastando: começo (onde o dedo desceu), cada movimento, fim. */
    fun dragStart(at: Offset, afterTap: Boolean)
    fun drag(at: Offset, delta: Offset)
    fun dragEnd()
    /** Dois dedos deslizando juntos: rolagem em "cliques" de roda. */
    fun scroll(notchesX: Int, notchesY: Int)
    /** Pinça: zoom local (não vai ao host). `panDelta` = deslocamento do centro. */
    fun zoom(factor: Float, centroid: Offset, panDelta: Offset)
}

private enum class TwoMode { Undecided, Scroll, Zoom }

/**
 * Reconhecedor de gestos da sessão (o mesmo para os modos Direto e Touchpad; a
 * tela decide o efeito de cada um):
 * - toque = clique; dois dedos tocando = clique direito; pressionar = clique direito;
 * - arrastar com um dedo = arrastar (Direto) ou mover o cursor (Touchpad);
 * - tocar e logo pressionar-arrastar (toque duplo + arrastar) = arrastar no Touchpad;
 * - dois dedos deslizando = rolagem; pinça = zoom local da imagem.
 */
fun Modifier.sessionGestures(target: GestureTarget, config: ViewConfiguration): Modifier =
    pointerInput(target) {
        val slop = config.touchSlop
        val longPressMs = config.longPressTimeoutMillis
        val doubleTapMs = config.doubleTapTimeoutMillis
        val scrollStepPx = 48.dp.toPx()
        var lastTapUpAt = 0L

        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val downAt = down.uptimeMillis
            val start = down.position
            val afterTap = downAt - lastTapUpAt < doubleTapMs
            var last = start
            var maxPointers = 1
            var moved = false
            var dragging = false
            var longPressed = false
            var two = TwoMode.Undecided
            var twoStartCentroid = Offset.Zero
            var twoStartSpan = 0f
            var prevCentroid = Offset.Zero
            var prevSpan = 0f
            var scrollAccX = 0f
            var scrollAccY = 0f
            down.consume()

            while (true) {
                val waitLong = !moved && !longPressed && maxPointers == 1
                val event = if (waitLong) {
                    // Tempo restante até o "pressionar", medido no relógio dos eventos.
                    val waited = elapsedSince(downAt, currentEvent.changes)
                    withTimeoutOrNull(maxOf(1L, longPressMs - waited)) { awaitPointerEvent() }
                } else {
                    awaitPointerEvent()
                }
                if (event == null) {
                    longPressed = true
                    target.longPress(start)
                    continue
                }
                val pressed = event.changes.filter { it.pressed }
                if (pressed.isEmpty()) {
                    if (dragging) target.dragEnd()
                    if (!moved && !longPressed) {
                        if (maxPointers >= 2) {
                            target.twoFingerTap(start)
                        } else {
                            target.tap(start)
                            lastTapUpAt = event.changes.firstOrNull()?.uptimeMillis ?: downAt
                        }
                    }
                    event.changes.forEach(PointerInputChange::consume)
                    break
                }
                if (pressed.size > maxPointers) {
                    maxPointers = pressed.size
                    if (pressed.size >= 2) {
                        if (dragging) {
                            target.dragEnd()
                            dragging = false
                        }
                        twoStartCentroid = centroid(pressed)
                        twoStartSpan = span(pressed)
                        prevCentroid = twoStartCentroid
                        prevSpan = twoStartSpan
                    }
                }
                if (pressed.size >= 2) {
                    val c = centroid(pressed)
                    val s = span(pressed)
                    if (two == TwoMode.Undecided) {
                        val spanChange = abs(s - twoStartSpan)
                        val move = (c - twoStartCentroid).getDistance()
                        if (spanChange > slop * 1.5f && spanChange > move * 0.8f) {
                            two = TwoMode.Zoom
                            moved = true
                        } else if (move > slop) {
                            two = TwoMode.Scroll
                            moved = true
                        }
                    }
                    when (two) {
                        TwoMode.Zoom -> {
                            if (prevSpan > 0f) target.zoom(s / prevSpan, c, c - prevCentroid)
                        }
                        TwoMode.Scroll -> {
                            val d = c - prevCentroid
                            // Dedos para cima = conteúdo sobe = rolar para baixo (como no touchpad).
                            scrollAccY += d.y
                            scrollAccX += d.x
                            val ny = (scrollAccY / scrollStepPx).toInt()
                            val nx = (scrollAccX / scrollStepPx).toInt()
                            if (ny != 0 || nx != 0) {
                                target.scroll(-nx, ny)
                                scrollAccY -= ny * scrollStepPx
                                scrollAccX -= nx * scrollStepPx
                            }
                        }
                        TwoMode.Undecided -> Unit
                    }
                    prevCentroid = c
                    prevSpan = s
                } else if (maxPointers == 1) {
                    val p = pressed.first().position
                    if (!moved && (p - start).getDistance() > slop) {
                        moved = true
                        if (!longPressed) {
                            dragging = true
                            target.dragStart(start, afterTap)
                        }
                    }
                    if (dragging && p != last) target.drag(p, p - last)
                    last = p
                }
                event.changes.forEach(PointerInputChange::consume)
            }
        }
    }

private fun elapsedSince(downAt: Long, changes: List<PointerInputChange>): Long {
    val now = changes.maxOfOrNull { it.uptimeMillis } ?: downAt
    return now - downAt
}

private fun centroid(ps: List<PointerInputChange>): Offset {
    var x = 0f
    var y = 0f
    ps.forEach { x += it.position.x; y += it.position.y }
    return Offset(x / ps.size, y / ps.size)
}

private fun span(ps: List<PointerInputChange>): Float {
    if (ps.size < 2) return 0f
    val a = ps[0].position
    val b = ps[1].position
    return hypot(a.x - b.x, a.y - b.y)
}
