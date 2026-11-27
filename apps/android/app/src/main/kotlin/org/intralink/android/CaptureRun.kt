package org.intralink.android

/**
 * One process-local, non-persistent diagnostic lease. No projection token,
 * image bytes or passphrase is stored here. Stale stop actions are isolated.
 */
internal object CaptureRun {
    enum class Phase { IDLE, STARTING, RUNNING, STOPPING, COMPLETE, STOPPED, SYSTEM_STOP, RESIZED, NO_OUTPUT, ERROR }

    data class Snapshot(
        val id: Long = 0,
        val phase: Phase = Phase.IDLE,
        val codec: String = "",
        val width: Int = 0,
        val height: Int = 0,
        val fps: Int = 0,
        val buffers: Long = 0,
        val bytes: Long = 0,
        val elapsedMs: Long = 0
    )

    private var nextId = 1L
    private var current = Snapshot()
    private var claimed = false

    @Synchronized
    fun reserve(): Long {
        if (current.phase in setOf(Phase.STARTING, Phase.RUNNING, Phase.STOPPING)) return 0
        if (nextId == Long.MAX_VALUE) return 0
        val id = nextId++
        current = Snapshot(id, Phase.STARTING)
        claimed = false
        return id
    }

    @Synchronized
    fun claim(id: Long): Boolean {
        if (current.id != id || current.phase != Phase.STARTING || claimed) return false
        claimed = true
        return true
    }

    @Synchronized
    fun active(id: Long): Boolean =
        current.id == id && current.phase in setOf(Phase.STARTING, Phase.RUNNING)

    @Synchronized
    fun cancel(id: Long) {
        if (active(id)) current = current.copy(phase = Phase.STOPPING)
    }

    @Synchronized
    fun update(value: Snapshot) {
        if (current.id == value.id && active(value.id) && claimed) current = value.copy(phase = Phase.RUNNING)
    }

    @Synchronized
    fun finish(id: Long, phase: Phase, value: Snapshot? = null) {
        if (current.id != id || current.phase !in setOf(Phase.STARTING, Phase.RUNNING, Phase.STOPPING)) return
        require(phase !in setOf(Phase.IDLE, Phase.STARTING, Phase.RUNNING, Phase.STOPPING))
        current = (value ?: current).copy(id = id, phase = phase)
        claimed = false
    }

    @Synchronized
    fun snapshot(): Snapshot = current
}
