package com.hyperlink.companion.ui

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** A computer seen on the local network. `id` is the mDNS service name (stable). */
data class Computer(
    val id: String,
    val name: String,
    val ip: String,
    val port: Int,
    val paired: Boolean,
)

/** Where the link to a computer stands — one of these is always true. */
sealed interface LinkPhase {
    /** Not connected; looking for computers. */
    data object Searching : LinkPhase
    data class Connecting(val computer: String) : LinkPhase
    /** First-time pairing: show `code`; `confirmed` once the user tapped Pair. */
    data class Pairing(val computer: String, val code: Int, val confirmed: Boolean) : LinkPhase
    data class Connected(val computer: String) : LinkPhase
    /** Something went wrong; `message` says what and what to do next. */
    data class Problem(val computer: String, val message: String, val canRetry: Boolean) : LinkPhase
}

data class Permissions(
    val notifications: Boolean = false,
    val doNotDisturb: Boolean = false,
    val control: Boolean = false,
    val files: Boolean = false,
    val postNotifications: Boolean = true,
)

data class UiState(
    val onboarded: Boolean = false,
    val phase: LinkPhase = LinkPhase.Searching,
    val computers: List<Computer> = emptyList(),
    val searchingSinceMs: Long = System.currentTimeMillis(),
    val mirroring: Boolean = false,
    val permissions: Permissions = Permissions(),
    val pairedComputers: List<String> = emptyList(),
    val log: List<String> = emptyList(),
)

/**
 * The single source of UI state. MainActivity's service callbacks write here;
 * the Compose screens only read it and call back through [UiActions].
 */
class LinkStore(context: Context) {
    private val prefs = context.getSharedPreferences("hyperlink_ui", Context.MODE_PRIVATE)
    private val _state = MutableStateFlow(
        UiState(
            onboarded = prefs.getBoolean(KEY_ONBOARDED, false),
            pairedComputers = prefs.getStringSet(KEY_PAIRED, emptySet())!!.sorted(),
        ),
    )
    val state: StateFlow<UiState> = _state.asStateFlow()

    fun update(transform: (UiState) -> UiState) = _state.update(transform)

    fun setOnboarded() {
        prefs.edit().putBoolean(KEY_ONBOARDED, true).apply()
        update { it.copy(onboarded = true) }
    }

    fun rememberPaired(name: String) {
        val set = prefs.getStringSet(KEY_PAIRED, emptySet())!!.toMutableSet().apply { add(name) }
        prefs.edit().putStringSet(KEY_PAIRED, set).apply()
        update { s ->
            s.copy(
                pairedComputers = set.sorted(),
                computers = s.computers.map { if (it.name == name) it.copy(paired = true) else it },
            )
        }
    }

    fun forgetPaired(name: String) {
        val set = prefs.getStringSet(KEY_PAIRED, emptySet())!!.toMutableSet().apply { remove(name) }
        prefs.edit().putStringSet(KEY_PAIRED, set).apply()
        update { s ->
            s.copy(
                pairedComputers = set.sorted(),
                computers = s.computers.map { if (it.name == name) it.copy(paired = false) else it },
            )
        }
    }

    fun isPaired(name: String) = name in _state.value.pairedComputers

    fun upsertComputer(computer: Computer) = update { s ->
        val others = s.computers.filterNot { it.id == computer.id }
        s.copy(computers = (others + computer).sortedWith(compareByDescending<Computer> { it.paired }.thenBy { it.name }))
    }

    fun removeComputer(id: String) = update { s -> s.copy(computers = s.computers.filterNot { it.id == id }) }

    /** Developer-facing activity log, shown under Settings → Activity log. */
    fun log(message: String) = update { s ->
        val line = "${TIME.format(Date())}  $message"
        s.copy(log = (s.log + line).takeLast(300))
    }

    companion object {
        private const val KEY_ONBOARDED = "onboarded"
        private const val KEY_PAIRED = "paired_computers"
        private val TIME = SimpleDateFormat("HH:mm:ss", Locale.US)
    }
}

/** What the screens can ask for. Implemented by MainActivity. */
interface UiActions {
    fun finishOnboarding()
    fun connect(computer: Computer)
    fun confirmPairing()
    fun cancelPairing()
    fun retry()
    fun disconnect()
    fun startMirroring()
    fun stopMirroring()
    fun openPermission(kind: PermissionKind)
    fun forgetComputer(name: String)
}

enum class PermissionKind { Notifications, DoNotDisturb, Control, Files, PostNotifications }
