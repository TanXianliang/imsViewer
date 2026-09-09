package com.imsviewer.android.domain

import com.imsviewer.android.domain.model.RemoteEntry
import java.util.PriorityQueue

internal data class RemoteThumbnailCandidate(
    val index: Int,
    val priority: Int,
    val entry: RemoteEntry
)

internal class RemoteThumbnailQueue {
    private val pending = PriorityQueue(
        compareBy<RemoteThumbnailCandidate> { it.priority }
            .thenBy { it.index }
    )
    private var inFlightPath: String? = null

    @Synchronized
    fun replace(candidates: Collection<RemoteThumbnailCandidate>) {
        pending.clear()
        candidates
            .asSequence()
            .filterNot { it.entry.path == inFlightPath }
            .forEach(pending::add)
    }

    @Synchronized
    fun poll(): RemoteEntry? {
        val candidate = pending.poll() ?: return null
        inFlightPath = candidate.entry.path
        return candidate.entry
    }

    @Synchronized
    fun complete(path: String) {
        if (inFlightPath == path) inFlightPath = null
    }

    @Synchronized
    fun hasPending(): Boolean = pending.isNotEmpty()
}
