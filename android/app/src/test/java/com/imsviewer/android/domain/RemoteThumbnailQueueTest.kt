package com.imsviewer.android.domain

import com.imsviewer.android.domain.model.RemoteEntry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteThumbnailQueueTest {
    @Test
    fun replacementReprioritizesPendingItemsWithoutDuplicatingInFlightItem() {
        val queue = RemoteThumbnailQueue()
        val first = entry("/first.jpg")
        val oldPending = entry("/old.jpg")
        val newlyVisible = entry("/visible.jpg")

        queue.replace(
            listOf(
                candidate(first, index = 0, priority = 0),
                candidate(oldPending, index = 1, priority = 1)
            )
        )
        assertEquals(first, queue.poll())

        queue.replace(
            listOf(
                candidate(first, index = 0, priority = 0),
                candidate(newlyVisible, index = 8, priority = 0)
            )
        )

        assertEquals(newlyVisible, queue.poll())
        assertNull(queue.poll())
    }

    @Test
    fun completedItemCanBeScheduledAgainWhenStillMissing() {
        val queue = RemoteThumbnailQueue()
        val image = entry("/image.jpg")

        queue.replace(listOf(candidate(image, index = 3, priority = 0)))
        assertEquals(image, queue.poll())
        queue.complete(image.path)
        queue.replace(listOf(candidate(image, index = 3, priority = 0)))

        assertEquals(image, queue.poll())
    }

    @Test
    fun visibleItemsArePolledBeforePrefetchItems() {
        val queue = RemoteThumbnailQueue()
        val prefetch = entry("/prefetch.jpg")
        val visible = entry("/visible.jpg")

        queue.replace(
            listOf(
                candidate(prefetch, index = 1, priority = 2),
                candidate(visible, index = 5, priority = 0)
            )
        )

        assertTrue(queue.hasPending())
        assertEquals(visible, queue.poll())
        assertEquals(prefetch, queue.poll())
        assertFalse(queue.hasPending())
    }

    private fun entry(path: String) = RemoteEntry(
        name = path.substringAfterLast('/'),
        path = path,
        isDirectory = false,
        sizeBytes = 1L
    )

    private fun candidate(entry: RemoteEntry, index: Int, priority: Int) =
        RemoteThumbnailCandidate(index = index, priority = priority, entry = entry)
}
