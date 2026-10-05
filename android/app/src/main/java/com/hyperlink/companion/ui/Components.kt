package com.hyperlink.companion.ui

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.foundation.clickable
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.hyperlink.companion.R

/** True when the user turned animations off (Settings → Accessibility → Remove animations). */
@Composable
fun reduceMotion(): Boolean {
    val resolver = LocalContext.current.contentResolver
    return remember {
        android.provider.Settings.Global.getFloat(
            resolver,
            android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,
            1f,
        ) == 0f
    }
}

/** The app backdrop: two soft light fields — the "link" — on the base color. Static. */
@Composable
fun Backdrop(modifier: Modifier = Modifier, content: @Composable BoxScope.() -> Unit) {
    val c = LocalHl.current
    Box(modifier.fillMaxSize().background(c.backdrop)) {
        Canvas(Modifier.fillMaxSize()) {
            drawRect(
                Brush.radialGradient(
                    listOf(c.glowA, Color.Transparent),
                    center = Offset(size.width * 0.1f, -size.height * 0.04f),
                    radius = size.maxDimension * 0.62f,
                ),
            )
            drawRect(
                Brush.radialGradient(
                    listOf(c.glowB, Color.Transparent),
                    center = Offset(size.width * 1.05f, size.height * 0.92f),
                    radius = size.maxDimension * 0.55f,
                ),
            )
        }
        content()
    }
}

/** A frosted surface: translucent fill and a rim that's brighter on top. */
@Composable
fun GlassCard(
    modifier: Modifier = Modifier,
    padding: Dp = Space.xl,
    radius: Dp = Radius.lg,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalHl.current
    val shape = RoundedCornerShape(radius)
    Column(
        modifier
            .fillMaxWidth()
            .shadow(
                elevation = if (c.isDark) 0.dp else 18.dp,
                shape = shape,
                ambientColor = Color(0xFF1C2A5A).copy(alpha = 0.18f),
                spotColor = Color(0xFF1C2A5A).copy(alpha = 0.18f),
            )
            .clip(shape)
            .background(c.glassFill)
            .border(1.dp, Brush.verticalGradient(listOf(c.glassRimTop, c.glassRimBottom)), shape)
            .padding(padding),
        content = content,
    )
}

/** The HyperLink mark on its tile. `tone` picks the tile: brand, success, or idle. */
enum class TileTone { Brand, Success, Idle }

@Composable
fun BrandTile(size: Dp = 88.dp, tone: TileTone = TileTone.Brand, pulse: Boolean = false) {
    val c = LocalHl.current
    val still = reduceMotion()
    val scale = if (pulse && !still) {
        val t = rememberInfiniteTransition(label = "tile")
        t.animateFloat(
            initialValue = 1f,
            targetValue = 1.05f,
            animationSpec = infiniteRepeatable(tween(1100, easing = FastOutSlowInEasing), RepeatMode.Reverse),
            label = "tileScale",
        ).value
    } else {
        1f
    }
    val shape = RoundedCornerShape(size * 0.3f)
    val (fill, glow) = when (tone) {
        TileTone.Brand -> Brush.verticalGradient(listOf(Brand.BlueHi, Brand.BlueLo)) to Brand.Blue
        TileTone.Success -> Brush.verticalGradient(listOf(Color(0xFF3CC48F), Color(0xFF1F9D6D))) to Brand.Success
        TileTone.Idle -> Brush.verticalGradient(listOf(c.glassFill, c.glassFill)) to Color.Transparent
    }
    Box(
        Modifier
            .scale(scale)
            .size(size)
            .shadow(if (tone == TileTone.Idle) 0.dp else 24.dp, shape, ambientColor = glow, spotColor = glow)
            .clip(shape)
            .background(fill)
            .then(if (tone == TileTone.Idle) Modifier.border(1.dp, c.divider, shape) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        Image(
            painter = painterResource(R.drawable.ic_hyperlink_mark),
            contentDescription = null,
            colorFilter = ColorFilter.tint(if (tone == TileTone.Idle) c.textMuted else Color.White),
            modifier = Modifier.size(size * 1.12f),
        )
    }
}

/** Filled pill for the one primary action on a screen. */
@Composable
fun PrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    busy: Boolean = false,
    color: Color = Brand.Blue,
    icon: ImageVector? = null,
) {
    val shape = RoundedCornerShape(50)
    Row(
        modifier
            .defaultMinSize(minHeight = 56.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(if (enabled) color else color.copy(alpha = 0.4f))
            .clickable(enabled = enabled && !busy, role = Role.Button, onClick = onClick)
            .padding(horizontal = Space.xl, vertical = Space.md),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) {
            CircularProgressIndicator(Modifier.size(18.dp), color = Color.White, strokeWidth = 2.dp)
            Spacer(Modifier.width(Space.md))
        } else if (icon != null) {
            Icon(icon, contentDescription = null, tint = Color.White, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(Space.sm))
        }
        Text(text, style = Type.label, color = Color.White)
    }
}

/** Low-emphasis text button. */
@Composable
fun QuietButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, color: Color? = null) {
    val c = LocalHl.current
    Box(
        modifier
            .heightIn(min = 48.dp)
            .clip(RoundedCornerShape(50))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = Space.lg, vertical = Space.md),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = Type.label, color = color ?: c.accentText)
    }
}

/** Small tonal button used inside rows ("Allow", "Turn on"). */
@Composable
fun RowAction(text: String, onClick: () -> Unit) {
    Box(
        Modifier
            .heightIn(min = 40.dp)
            .clip(RoundedCornerShape(50))
            .background(Brand.Blue.copy(alpha = 0.16f))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = Space.lg, vertical = Space.sm),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = Type.caption.copy(fontWeight = Type.label.fontWeight), color = LocalHl.current.accentText)
    }
}

@Composable
fun StatusDot(color: Color, pulsing: Boolean = false) {
    val still = reduceMotion()
    val alpha = if (pulsing && !still) {
        rememberInfiniteTransition(label = "dot").animateFloat(
            initialValue = 1f,
            targetValue = 0.35f,
            animationSpec = infiniteRepeatable(tween(900), RepeatMode.Reverse),
            label = "dotAlpha",
        ).value
    } else {
        1f
    }
    Box(Modifier.size(8.dp).clip(CircleShape).background(color.copy(alpha = alpha)))
}

/** "● Connected" style chip. */
@Composable
fun StatusChip(text: String, dot: Color, pulsing: Boolean = false) {
    val c = LocalHl.current
    Row(
        Modifier
            .clip(RoundedCornerShape(50))
            .background(c.glassFill)
            .border(1.dp, c.divider, RoundedCornerShape(50))
            .padding(horizontal = Space.md, vertical = 6.dp)
            .semantics(mergeDescendants = true) { contentDescription = text },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StatusDot(dot, pulsing)
        Spacer(Modifier.width(Space.sm))
        Text(text, style = Type.caption.copy(fontWeight = Type.label.fontWeight), color = c.text)
    }
}

@Composable
fun SectionLabel(text: String) {
    Text(
        text.uppercase(),
        style = Type.caption.copy(fontWeight = Type.label.fontWeight, letterSpacing = Type.caption.letterSpacing),
        color = LocalHl.current.textMuted,
        modifier = Modifier.padding(start = Space.xs, bottom = Space.sm),
    )
}

/** One feature line in a glass list: icon, title, status/explanation, optional action. */
@Composable
fun FeatureRow(
    icon: ImageVector,
    title: String,
    detail: String,
    ok: Boolean,
    action: String? = null,
    onAction: (() -> Unit)? = null,
) {
    val c = LocalHl.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(vertical = Space.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.size(40.dp).clip(RoundedCornerShape(Radius.sm))
                .background(if (ok) Brand.Blue.copy(alpha = 0.14f) else c.divider),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, contentDescription = null, tint = if (ok) c.accentText else c.textMuted, modifier = Modifier.size(22.dp))
        }
        Spacer(Modifier.width(Space.md))
        // With large text, a trailing button squeezes the text into a narrow
        // column; put it under the text instead.
        val stack = LocalDensity.current.fontScale > 1.25f
        Column(Modifier.weight(1f)) {
            Text(title, style = Type.heading, color = c.text)
            Text(detail, style = Type.caption, color = c.textMuted)
            if (stack && action != null && onAction != null) {
                Spacer(Modifier.height(Space.sm))
                RowAction(action, onAction)
            }
        }
        if (!stack && action != null && onAction != null) {
            Spacer(Modifier.width(Space.sm))
            RowAction(action, onAction)
        }
    }
}

@Composable
fun Divider() {
    Box(Modifier.padding(start = 52.dp).fillMaxWidth().height(1.dp).background(LocalHl.current.divider))
}

@Composable
fun CenteredText(text: String, style: androidx.compose.ui.text.TextStyle, color: Color, modifier: Modifier = Modifier) {
    Text(text, style = style, color = color, textAlign = TextAlign.Center, modifier = modifier.fillMaxWidth())
}
