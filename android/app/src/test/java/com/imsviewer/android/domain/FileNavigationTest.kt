package com.imsviewer.android.domain

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FileNavigationTest {
    @Test
    fun nextFromLastWrapsToFirst() {
        assertEquals(0, wrappedAdjacentIndex(currentIndex = 2, itemCount = 3, direction = 1))
    }

    @Test
    fun previousFromFirstWrapsToLast() {
        assertEquals(2, wrappedAdjacentIndex(currentIndex = 0, itemCount = 3, direction = -1))
    }

    @Test
    fun singleItemHasNoAdjacentTarget() {
        assertNull(wrappedAdjacentIndex(currentIndex = 0, itemCount = 1, direction = 1))
    }

    @Test
    fun invalidCurrentIndexHasNoAdjacentTarget() {
        assertNull(wrappedAdjacentIndex(currentIndex = -1, itemCount = 3, direction = 1))
    }
}
