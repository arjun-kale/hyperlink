package com.hyperlink.companion.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.DoNotDisturbOn
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.ScreenShare
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.StopScreenShare
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.hyperlink.companion.BuildConfig
import kotlinx.coroutines.delay

/** Formats a pairing code the way both devices show it: "482 913". */
fun formatCode(code: Int): String = "%06d".format(code).let { "${it.substring(0, 3)} ${it.substring(3)}" }

private enum class Route { Welcome, Find, Pairing, Linked }

@Composable
fun HyperLinkApp(state: UiState, actions: UiActions) {
    var showSettings by rememberSaveable { mutableStateOf(false) }
    val route = when {
        !state.onboarded -> Route.Welcome
        state.phase is LinkPhase.Pairing -> Route.Pairing
        state.phase is LinkPhase.Connected -> Route.Linked
        else -> Route.Find
    }
    val still = reduceMotion()
    Backdrop {
        AnimatedContent(
            targetState = if (showSettings) null else route,
            transitionSpec = {
                val d = if (still) 0 else 260
                fadeIn(tween(d)) togetherWith fadeOut(tween(d))
            },
            label = "route",
        ) { target ->
            when (target) {
                null -> SettingsScreen(state, actions, onBack = { showSettings = false })
                Route.Welcome -> WelcomeScreen(actions)
                Route.Find -> FindScreen(state, actions, onSettings = { showSettings = true })
                Route.Pairing -> PairingScreen(state.phase as? LinkPhase.Pairing ?: return@AnimatedContent, actions)
                Route.Linked -> LinkedScreen(state, actions, onSettings = { showSettings = true })
            }
        }
    }
}

/** Common screen frame: system-bar safe, scrollable, with the screen gutter. */
@Composable
private fun ScreenColumn(
    topBar: (@Composable () -> Unit)? = null,
    bottom: (@Composable () -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    Column(Modifier.fillMaxSize().systemBarsPadding()) {
        topBar?.invoke()
        Column(
            Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(horizontal = Space.gutter),
        ) { content() }
        if (bottom != null) {
            Column(Modifier.fillMaxWidth().padding(horizontal = Space.gutter, vertical = Space.lg)) { bottom() }
        }
    }
}

@Composable
private fun TopBar(title: String? = null, onBack: (() -> Unit)? = null, onSettings: (() -> Unit)? = null) {
    val c = LocalHl.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = Space.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, contentDescription = "Back", tint = c.text)
            }
        } else {
            Spacer(Modifier.width(Space.lg))
        }
        Text(
            title ?: "HyperLink",
            style = Type.heading,
            color = c.text,
            modifier = Modifier.weight(1f).semantics { heading() },
        )
        if (onSettings != null) {
            IconButton(onClick = onSettings) {
                Icon(Icons.Rounded.Settings, contentDescription = "Settings", tint = c.textMuted)
            }
        }
    }
}

// ── Welcome ──────────────────────────────────────────────────────────────

@Composable
private fun WelcomeScreen(actions: UiActions) {
    val c = LocalHl.current
    ScreenColumn(bottom = {
        PrimaryButton("Get started", onClick = actions::finishOnboarding)
    }) {
        Spacer(Modifier.height(Space.xxl))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { BrandTile(80.dp) }
        Spacer(Modifier.height(Space.xl))
        CenteredText("Your phone,\non your computer", Type.display, c.text, Modifier.semantics { heading() })
        Spacer(Modifier.height(Space.md))
        CenteredText(
            "HyperLink connects this phone to your Linux computer over your own Wi-Fi.",
            Type.body,
            c.textMuted,
        )
        Spacer(Modifier.height(Space.xxl))
        GlassCard(padding = Space.lg) {
            WelcomePoint(Icons.Rounded.ScreenShare, "See and control your screen", "Use your phone with your computer's mouse and keyboard.")
            Spacer(Modifier.height(Space.md))
            WelcomePoint(Icons.Rounded.Notifications, "Never miss a notification", "Your phone's alerts appear on your computer.")
            Spacer(Modifier.height(Space.md))
            WelcomePoint(Icons.Rounded.ContentPaste, "Copy here, paste there", "Clipboard and files move between your devices.")
        }
        Spacer(Modifier.height(Space.xl))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Rounded.Lock, contentDescription = null, tint = c.textMuted, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(Space.sm))
            Text("No account. Nothing leaves your Wi-Fi.", style = Type.caption, color = c.textMuted)
        }
        Spacer(Modifier.height(Space.xl))
    }
}

@Composable
private fun WelcomePoint(icon: ImageVector, title: String, detail: String) {
    val c = LocalHl.current
    Row(verticalAlignment = Alignment.Top) {
        Box(
            Modifier.size(40.dp).clip(RoundedCornerShape(Radius.sm)).background(Brand.Blue.copy(alpha = 0.14f)),
            contentAlignment = Alignment.Center,
        ) { Icon(icon, contentDescription = null, tint = c.accentText, modifier = Modifier.size(22.dp)) }
        Spacer(Modifier.width(Space.md))
        Column {
            Text(title, style = Type.heading, color = c.text)
            Text(detail, style = Type.caption, color = c.textMuted)
        }
    }
}

// ── Find your computer ───────────────────────────────────────────────────

@Composable
private fun FindScreen(state: UiState, actions: UiActions, onSettings: () -> Unit) {
    val c = LocalHl.current
    val phase = state.phase
    // After a while with nothing found, explain the usual causes.
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000)
            now = System.currentTimeMillis()
        }
    }
    val waitedLong = now - state.searchingSinceMs > 12_000

    ScreenColumn(topBar = { TopBar(onSettings = onSettings) }) {
        Spacer(Modifier.height(Space.xxl))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
            BrandTile(
                80.dp,
                tone = if (phase is LinkPhase.Problem) TileTone.Idle else TileTone.Brand,
                pulse = phase is LinkPhase.Searching || phase is LinkPhase.Connecting,
            )
        }
        Spacer(Modifier.height(Space.xl))
        val (title, subtitle) = when (phase) {
            is LinkPhase.Connecting -> "Connecting to ${phase.computer}" to "This only takes a moment."
            is LinkPhase.Problem -> "Couldn't connect" to phase.message
            else -> if (state.pairedComputers.isEmpty()) {
                "Find your computer" to "Open HyperLink on your computer and choose Start Pairing. It will show up here."
            } else {
                "Looking for your computer" to "HyperLink connects on its own when your computer is on the same Wi-Fi."
            }
        }
        CenteredText(title, Type.title, c.text, Modifier.semantics { heading() })
        Spacer(Modifier.height(Space.sm))
        CenteredText(subtitle, Type.body, c.textMuted)

        if (phase is LinkPhase.Problem && phase.canRetry) {
            Spacer(Modifier.height(Space.lg))
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                QuietButton("Try again", onClick = actions::retry)
            }
        }

        Spacer(Modifier.height(Space.xxl))
        Row(verticalAlignment = Alignment.CenterVertically) {
            SectionLabel("Computers on this Wi-Fi")
            Spacer(Modifier.weight(1f))
            if (phase is LinkPhase.Searching) {
                CircularProgressIndicator(
                    Modifier.size(14.dp).padding(bottom = Space.xs),
                    strokeWidth = 2.dp,
                    color = c.textMuted,
                )
                Spacer(Modifier.width(Space.xs))
            }
        }
        if (state.computers.isEmpty()) {
            GlassCard(padding = Space.xl) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.Wifi, contentDescription = null, tint = c.textMuted)
                    Spacer(Modifier.width(Space.md))
                    Text("Searching…", style = Type.body, color = c.textMuted)
                }
                if (waitedLong) {
                    Spacer(Modifier.height(Space.lg))
                    Text("Not showing up?", style = Type.heading, color = c.text)
                    Spacer(Modifier.height(Space.xs))
                    Text(
                        "• Make sure HyperLink is open on your computer.\n" +
                            "• Check both devices are on the same Wi-Fi network.\n" +
                            "• Guest or public Wi-Fi often blocks devices from seeing each other.",
                        style = Type.caption,
                        color = c.textMuted,
                    )
                }
            }
        } else {
            GlassCard(padding = Space.sm) {
                state.computers.forEachIndexed { i, computer ->
                    if (i > 0) Divider()
                    ComputerRow(
                        computer,
                        busy = phase is LinkPhase.Connecting && phase.computer == computer.name,
                        onClick = { actions.connect(computer) },
                    )
                }
            }
        }
        Spacer(Modifier.height(Space.xl))
    }
}

@Composable
private fun ComputerRow(computer: Computer, busy: Boolean, onClick: () -> Unit) {
    val c = LocalHl.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(RoundedCornerShape(Radius.md))
            .clickable(enabled = !busy, onClick = onClick)
            .padding(horizontal = Space.md, vertical = Space.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.size(40.dp).clip(RoundedCornerShape(Radius.sm)).background(Brand.Blue.copy(alpha = 0.14f)),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.Laptop, contentDescription = null, tint = c.accentText, modifier = Modifier.size(22.dp)) }
        Spacer(Modifier.width(Space.md))
        Column(Modifier.weight(1f)) {
            Text(computer.name, style = Type.heading, color = c.text)
            Text(
                if (computer.paired) "Paired · tap to connect" else "Tap to pair",
                style = Type.caption,
                color = c.textMuted,
            )
        }
        if (busy) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = c.accentText)
        } else {
            Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, contentDescription = null, tint = c.textMuted)
        }
    }
}

// ── Pairing ──────────────────────────────────────────────────────────────

@Composable
private fun PairingScreen(phase: LinkPhase.Pairing, actions: UiActions) {
    val c = LocalHl.current
    BackHandler(onBack = actions::cancelPairing)
    ScreenColumn(
        topBar = { TopBar(title = "Pair with ${phase.computer}", onBack = actions::cancelPairing) },
        bottom = {
            PrimaryButton(
                if (phase.confirmed) "Waiting for your computer…" else "I picked this code — Pair",
                onClick = actions::confirmPairing,
                busy = phase.confirmed,
            )
            Spacer(Modifier.height(Space.xs))
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                QuietButton("Cancel", onClick = actions::cancelPairing, color = c.textMuted)
            }
        },
    ) {
        Spacer(Modifier.height(Space.xxl))
        CenteredText("Pick this code on your computer", Type.title, c.text, Modifier.semantics { heading() })
        Spacer(Modifier.height(Space.sm))
        CenteredText(
            "Your computer shows three codes. Choosing the matching one proves you're pairing with your own computer.",
            Type.body,
            c.textMuted,
        )
        Spacer(Modifier.height(Space.xxl))
        GlassCard(Modifier.fillMaxWidth(), padding = Space.xxl) {
            Text(
                formatCode(phase.code),
                style = Type.code.copy(fontFamily = FontFamily.Monospace),
                color = c.text,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )
        }
        Spacer(Modifier.height(Space.xl))
        Step(1, "On your computer, click the code that matches", done = phase.confirmed)
        Spacer(Modifier.height(Space.md))
        Step(2, "Then tap Pair below", done = false)
    }
}


@Composable
private fun Step(n: Int, text: String, done: Boolean) {
    val c = LocalHl.current
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            Modifier.size(28.dp).clip(RoundedCornerShape(50))
                .background(if (done) Brand.Success.copy(alpha = 0.2f) else Brand.Blue.copy(alpha = 0.16f)),
            contentAlignment = Alignment.Center,
        ) { Text(if (done) "✓" else "$n", style = Type.caption.copy(fontWeight = Type.label.fontWeight), color = if (done) Brand.Success else c.accentText) }
        Spacer(Modifier.width(Space.md))
        Text(text, style = Type.body, color = c.text)
    }
}

// ── Linked ───────────────────────────────────────────────────────────────

@Composable
private fun LinkedScreen(state: UiState, actions: UiActions, onSettings: () -> Unit) {
    val c = LocalHl.current
    val computer = (state.phase as? LinkPhase.Connected)?.computer ?: "your computer"
    val p = state.permissions
    ScreenColumn(topBar = { TopBar(onSettings = onSettings) }) {
        Spacer(Modifier.height(Space.lg))
        GlassCard(Modifier.fillMaxWidth()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                BrandTile(56.dp, tone = TileTone.Success)
                Spacer(Modifier.width(Space.lg))
                Column(Modifier.weight(1f)) {
                    Text("Linked with", style = Type.caption, color = c.textMuted)
                    Text(computer, style = Type.title, color = c.text, modifier = Modifier.semantics { heading() })
                }
            }
            Spacer(Modifier.height(Space.lg))
            Row {
                if (state.mirroring) {
                    StatusChip("Sharing your screen", Brand.Danger, pulsing = true)
                } else {
                    StatusChip("Connected", Brand.Success)
                }
            }
            Spacer(Modifier.height(Space.xl))
            if (state.mirroring) {
                PrimaryButton(
                    "Stop showing screen",
                    onClick = actions::stopMirroring,
                    color = Color(0xFF3A4152),
                    icon = Icons.Rounded.StopScreenShare,
                )
            } else {
                PrimaryButton("Show screen on PC", onClick = actions::startMirroring, icon = Icons.Rounded.ScreenShare)
            }
            if (!p.control) {
                Spacer(Modifier.height(Space.sm))
                Text(
                    "To control your phone from the computer, turn on “Control from PC” below.",
                    style = Type.caption,
                    color = c.textMuted,
                )
            }
        }

        Spacer(Modifier.height(Space.xl))
        val missing = listOf(p.notifications, p.control, p.files, p.doNotDisturb).count { !it }
        SectionLabel(if (missing == 0) "Everything's on" else "Finish setting up · $missing left")
        GlassCard(padding = Space.lg) {
            FeatureRow(
                Icons.Rounded.Notifications, "Notifications",
                if (p.notifications) "Showing on your computer" else "Allow HyperLink to read your notifications",
                ok = p.notifications,
                action = if (p.notifications) null else "Allow",
                onAction = { actions.openPermission(PermissionKind.Notifications) },
            )
            Divider()
            FeatureRow(
                Icons.Rounded.Mouse, "Control from PC",
                if (p.control) "Mouse and keyboard work on your phone" else "Turn on HyperLink in Accessibility settings",
                ok = p.control,
                action = if (p.control) null else "Turn on",
                onAction = { actions.openPermission(PermissionKind.Control) },
            )
            Divider()
            FeatureRow(
                Icons.Rounded.ContentPaste, "Clipboard",
                "Copy on one device, paste on the other",
                ok = true,
            )
            Divider()
            FeatureRow(
                Icons.Rounded.Folder, "Files",
                if (p.files) "Your computer can browse your storage" else "Allow access so your computer can browse files",
                ok = p.files,
                action = if (p.files) null else "Allow",
                onAction = { actions.openPermission(PermissionKind.Files) },
            )
            Divider()
            FeatureRow(
                Icons.Rounded.DoNotDisturbOn, "Do Not Disturb sync",
                if (p.doNotDisturb) "Muting one mutes both" else "Allow so muting one device mutes both",
                ok = p.doNotDisturb,
                action = if (p.doNotDisturb) null else "Allow",
                onAction = { actions.openPermission(PermissionKind.DoNotDisturb) },
            )
        }

        Spacer(Modifier.height(Space.xl))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
            QuietButton("Disconnect", onClick = actions::disconnect, color = c.textMuted)
        }
        Spacer(Modifier.height(Space.lg))
    }
}

// ── Settings ─────────────────────────────────────────────────────────────

@Composable
private fun SettingsScreen(state: UiState, actions: UiActions, onBack: () -> Unit) {
    val c = LocalHl.current
    var showLog by remember { mutableStateOf(false) }
    BackHandler { if (showLog) showLog = false else onBack() }
    if (showLog) {
        ScreenColumn(topBar = { TopBar("Activity log", onBack = { showLog = false }) }) {
            GlassCard(padding = Space.lg) {
                if (state.log.isEmpty()) {
                    Text("Nothing yet.", style = Type.caption, color = c.textMuted)
                }
                state.log.asReversed().forEach {
                    Text(it, style = Type.caption.copy(fontFamily = FontFamily.Monospace), color = c.textMuted)
                }
            }
            Spacer(Modifier.height(Space.xl))
        }
        return
    }
    ScreenColumn(topBar = { TopBar("Settings", onBack = onBack) }) {
        Spacer(Modifier.height(Space.lg))
        SectionLabel("Paired computers")
        GlassCard(padding = Space.lg) {
            if (state.pairedComputers.isEmpty()) {
                Text("None yet. Pair a computer from the main screen.", style = Type.body, color = c.textMuted)
            }
            state.pairedComputers.forEachIndexed { i, name ->
                if (i > 0) Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.Laptop, contentDescription = null, tint = c.textMuted)
                    Spacer(Modifier.width(Space.md))
                    Text(name, style = Type.heading, color = c.text, modifier = Modifier.weight(1f))
                    QuietButton("Forget", onClick = { actions.forgetComputer(name) }, color = Brand.Danger)
                }
            }
        }
        Spacer(Modifier.height(Space.sm))
        Text(
            "Forgetting hides a computer here. To stop it reconnecting, also remove this phone in HyperLink on that computer.",
            style = Type.caption,
            color = c.textMuted,
            modifier = Modifier.padding(horizontal = Space.xs),
        )

        Spacer(Modifier.height(Space.xl))
        SectionLabel("Help")
        GlassCard(padding = Space.sm) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(Radius.md))
                    .clickable { showLog = true }.padding(horizontal = Space.md),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Activity log", style = Type.heading, color = c.text)
                    Text("Technical details, for troubleshooting", style = Type.caption, color = c.textMuted)
                }
                Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, contentDescription = null, tint = c.textMuted)
            }
        }

        Spacer(Modifier.height(Space.xxl))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { BrandTile(48.dp) }
        Spacer(Modifier.height(Space.md))
        CenteredText("HyperLink ${BuildConfig.VERSION_NAME}", Type.caption, c.textMuted)
        CenteredText("Your phone and computer, over your own Wi-Fi.", Type.caption, c.textMuted)
        Spacer(Modifier.height(Space.xl))
    }
}
