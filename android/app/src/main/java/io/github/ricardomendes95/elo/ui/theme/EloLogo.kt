package io.github.ricardomendes95.elo.ui.theme

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Logo do Elo: dois elos de corrente entrelaçados, inclinados 45°. */
@Composable
fun EloLogo(size: Dp = 32.dp, color: Color = EloTheme.colors.accent, modifier: Modifier = Modifier) {
    Canvas(modifier.size(size)) {
        val u = this.size.minDimension / 108f
        val w = 48f * u
        val h = 20f * u
        val stroke = Stroke(width = 6f * u, join = StrokeJoin.Round)
        fun link(cx: Float, cy: Float) = rotate(-45f, pivot = Offset(cx * u, cy * u)) {
            drawRoundRect(
                color = color,
                topLeft = Offset(cx * u - w / 2, cy * u - h / 2),
                size = Size(w, h),
                cornerRadius = CornerRadius(h / 2, h / 2),
                style = stroke,
            )
        }
        link(42f, 66f)
        link(66f, 42f)
    }
}
