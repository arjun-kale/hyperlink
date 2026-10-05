package com.hyperlink.companion.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * HyperLink design tokens. Shared vocabulary with the Linux host
 * (linux/src/style.css): one brand blue for the link itself and primary
 * actions, semantic colors only for state, glass only on floating surfaces.
 */
object Brand {
    val Blue = Color(0xFF2366FF)
    val BlueHi = Color(0xFF3D7BFF)
    val BlueLo = Color(0xFF1C56F0)
    val Sky = Color(0xFF5AA9FF)
    val Success = Color(0xFF2FB380)
    val Warning = Color(0xFFE8A23A)
    val Danger = Color(0xFFE5484D)
}

@Immutable
data class HlColors(
    val isDark: Boolean,
    val backdrop: Color,
    val text: Color,
    val textMuted: Color,
    val glassFill: Color,
    val glassRimTop: Color,
    val glassRimBottom: Color,
    val divider: Color,
    val accentText: Color,
    val glowA: Color,
    val glowB: Color,
)

private val DarkColors = HlColors(
    isDark = true,
    backdrop = Color(0xFF070A12),
    text = Color(0xFFF2F4F8),
    textMuted = Color(0xFFA4ABBA),
    glassFill = Color.White.copy(alpha = 0.06f),
    glassRimTop = Color.White.copy(alpha = 0.22f),
    glassRimBottom = Color.White.copy(alpha = 0.04f),
    divider = Color.White.copy(alpha = 0.08f),
    accentText = Color(0xFF7FA6FF),
    glowA = Brand.Blue.copy(alpha = 0.34f),
    glowB = Brand.Sky.copy(alpha = 0.16f),
)

private val LightColors = HlColors(
    isDark = false,
    backdrop = Color(0xFFF4F6FB),
    text = Color(0xFF111827),
    textMuted = Color(0xFF5B6475),
    glassFill = Color.White.copy(alpha = 0.72f),
    glassRimTop = Color.White,
    glassRimBottom = Color(0xFF1C2A5A).copy(alpha = 0.08f),
    divider = Color(0xFF111827).copy(alpha = 0.07f),
    accentText = Brand.BlueLo,
    glowA = Brand.Blue.copy(alpha = 0.16f),
    glowB = Brand.Sky.copy(alpha = 0.14f),
)

/** 4dp-based spacing scale. */
object Space {
    val xs = 4.dp
    val sm = 8.dp
    val md = 12.dp
    val lg = 16.dp
    val xl = 24.dp
    val xxl = 32.dp
    val xxxl = 48.dp
    /** Side margin for every screen. */
    val gutter = 20.dp
}

object Radius {
    val sm = 12.dp
    val md = 20.dp
    val lg = 28.dp
}

/** Type scale: display → title → body → caption. System font (One UI Sans on Samsung). */
object Type {
    val display = TextStyle(fontSize = 32.sp, lineHeight = 38.sp, fontWeight = FontWeight.Bold, letterSpacing = (-0.5).sp)
    val title = TextStyle(fontSize = 22.sp, lineHeight = 28.sp, fontWeight = FontWeight.SemiBold)
    val heading = TextStyle(fontSize = 17.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold)
    val body = TextStyle(fontSize = 16.sp, lineHeight = 23.sp)
    val label = TextStyle(fontSize = 15.sp, lineHeight = 20.sp, fontWeight = FontWeight.SemiBold)
    val caption = TextStyle(fontSize = 13.sp, lineHeight = 18.sp)
    val code = TextStyle(fontSize = 44.sp, lineHeight = 52.sp, fontWeight = FontWeight.Bold, letterSpacing = 4.sp)
}

val LocalHl = staticCompositionLocalOf { DarkColors }

@Composable
fun HyperLinkTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    val colors = if (dark) DarkColors else LightColors
    val material = if (dark) {
        darkColorScheme(primary = Brand.Blue, onPrimary = Color.White, background = colors.backdrop, surface = Color(0xFF111522))
    } else {
        lightColorScheme(primary = Brand.Blue, onPrimary = Color.White, background = colors.backdrop, surface = Color.White)
    }
    CompositionLocalProvider(LocalHl provides colors) {
        MaterialTheme(colorScheme = material, typography = Typography(), content = content)
    }
}
