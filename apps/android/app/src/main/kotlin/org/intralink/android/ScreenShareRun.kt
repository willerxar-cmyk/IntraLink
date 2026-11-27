package org.intralink.android

/**
 * One foreground screen-only approval, held in memory until the service
 * claims it. The shared Rust registry also excludes concurrent diagnostics.
 * No secret is placed in Intent extras or saved instance state.
 */
internal object ScreenShareRun {
    enum class Phase { WAITING, STOPPING, COMPLETE, STOPPED, ERROR }
    data class Approval(
        val id: Long,
        val path: String,
        val request: String,
        val password: ByteArray,
        val width: Int,
        val height: Int,
        val density: Int,
        val navigation: Boolean
    )
    data class Snapshot(val id: Long, val phase: Phase, val result: String? = null)

    private var pending: Approval? = null
    private var state = Snapshot(0, Phase.STOPPED)
    private var claimed = false
    private var navigation = false
    private var attached = false

    @Synchronized
    fun reserve(path: String, request: String, password: ByteArray,
                width: Int, height: Int, density: Int, navigation: Boolean = false): Long {
        if (state.phase == Phase.WAITING || state.phase == Phase.STOPPING) return 0
        if (CaptureRun.snapshot().phase in setOf(CaptureRun.Phase.STARTING,
                CaptureRun.Phase.RUNNING, CaptureRun.Phase.STOPPING)) return 0
        val id = NativeBridge.reserveProbe()
        if (id == 0L) return 0
        pending = Approval(id, path, request, password, width, height, density, navigation)
        state = Snapshot(id, Phase.WAITING)
        claimed = false
        this.navigation = navigation
        attached = false
        return id
    }

    @Synchronized
    fun claim(id: Long): Approval? {
        if (!active(id) || claimed) return null
        val approval = pending ?: return null
        if (approval.id != id) return null
        claimed = true
        pending = null
        return approval
    }

    @Synchronized
    fun active(id: Long): Boolean = state.id == id && state.phase == Phase.WAITING

    /** Navigation is authorized only after the authenticated receiver demands capture. */
    @Synchronized
    fun markAttached(id: Long) {
        if (active(id) && claimed) attached = true
    }

    @Synchronized
    fun mayNavigate(id: Long): Boolean = active(id) && navigation && attached

    @Synchronized
    fun cancel(id: Long) {
        if (!active(id)) return
        pending?.password?.fill(0)
        pending = null
        NativeBridge.cancelProbe(id)
        state = Snapshot(id, if (claimed) Phase.STOPPING else Phase.STOPPED)
    }

    @Synchronized
    fun finish(id: Long, success: Boolean, result: String? = null) {
        if (state.id != id || state.phase !in setOf(Phase.WAITING, Phase.STOPPING)) return
        pending?.password?.fill(0)
        pending = null
        NativeBridge.cancelProbe(id)
        state = Snapshot(id, when {
            state.phase == Phase.STOPPING -> Phase.STOPPED
            success -> Phase.COMPLETE
            else -> Phase.ERROR
        }, result)
        claimed = false
        navigation = false
        attached = false
    }

    @Synchronized
    fun snapshot(): Snapshot = state
}
