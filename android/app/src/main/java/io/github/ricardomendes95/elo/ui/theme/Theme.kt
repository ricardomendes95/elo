package io.github.ricardomendes95.elo.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.shape.RoundedCornerShape

/** Tokens de cor do Elo (inclui os que o Material3 não tem: warn, ok, muted). */
data class EloColors(
    val bg: Color,
    val bar: Color,
    val surface: Color,
    val elevated: Color,
    val border: Color,
    val text: Color,
    val muted: Color,
    val accent: Color,
    val onAccent: Color,
    val accent2: Color,
    val warn: Color,
    val danger: Color,
    val ok: Color,
    val dark: Boolean,
)

val DarkEloColors = EloColors(
    bg = Color(0xFF0F141B), bar = Color(0xFF121821), surface = Color(0xFF181F29),
    elevated = Color(0xFF202935), border = Color(0xFF2C3745), text = Color(0xFFE7ECF2),
    muted = Color(0xFF98A4B3), accent = Color(0xFF2DD4BF), onAccent = Color(0xFF052420),
    accent2 = Color(0xFF8B97F8), warn = Color(0xFFECB44E), danger = Color(0xFFF26D72),
    ok = Color(0xFF4ADE80), dark = true,
)

val LightEloColors = EloColors(
    bg = Color(0xFFF5F7FA), bar = Color(0xFFFFFFFF), surface = Color(0xFFFFFFFF),
    elevated = Color(0xFFF0F2F6), border = Color(0xFFDDE2EA), text = Color(0xFF1A202C),
    muted = Color(0xFF647080), accent = Color(0xFF12B3A6), onAccent = Color(0xFFFFFFFF),
    accent2 = Color(0xFF4F55E0), warn = Color(0xFFC87A12), danger = Color(0xFFD1444A),
    ok = Color(0xFF1FA25A), dark = false,
)

val LocalEloColors = staticCompositionLocalOf { DarkEloColors }

/** Acesso curto: `EloTheme.colors.warn`. */
object EloTheme {
    val colors: EloColors
        @Composable @ReadOnlyComposable get() = LocalEloColors.current
}

/** [theme]: "dark", "light" ou "system". */
@Composable
fun EloTheme(theme: String = "dark", content: @Composable () -> Unit) {
    val dark = when (theme) {
        "light" -> false
        "system" -> isSystemInDarkTheme()
        else -> true
    }
    val c = if (dark) DarkEloColors else LightEloColors
    val scheme = if (dark) {
        darkColorScheme(
            primary = c.accent, onPrimary = c.onAccent, secondary = c.accent2, onSecondary = c.onAccent,
            background = c.bg, onBackground = c.text, surface = c.surface, onSurface = c.text,
            surfaceVariant = c.elevated, onSurfaceVariant = c.muted, outline = c.border,
            outlineVariant = c.border, error = c.danger, surfaceContainer = c.surface,
            surfaceContainerHigh = c.elevated, surfaceContainerHighest = c.elevated,
        )
    } else {
        lightColorScheme(
            primary = c.accent, onPrimary = c.onAccent, secondary = c.accent2, onSecondary = Color.White,
            background = c.bg, onBackground = c.text, surface = c.surface, onSurface = c.text,
            surfaceVariant = c.elevated, onSurfaceVariant = c.muted, outline = c.border,
            outlineVariant = c.border, error = c.danger, surfaceContainer = c.surface,
            surfaceContainerHigh = c.elevated, surfaceContainerHighest = c.elevated,
        )
    }
    val shapes = Shapes(
        small = RoundedCornerShape(8.dp), medium = RoundedCornerShape(12.dp), large = RoundedCornerShape(12.dp),
    )
    CompositionLocalProvider(LocalEloColors provides c) {
        MaterialTheme(colorScheme = scheme, shapes = shapes, content = content)
    }
}
