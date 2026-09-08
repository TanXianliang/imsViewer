package com.imsviewer.android.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class FitModeSwipeTest {
    @Test
    fun shortFastUpwardFlickOpensNextSubImage() {
        val action = resolveFitModeSwipe(
            axis = SwipeAxis.Vertical,
            dragX = 12f,
            dragY = -42f,
            velocityX = 200f,
            velocityY = -1_800f,
            horizontalDistanceThreshold = 120f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = true
        )

        assertEquals(FitModeSwipeAction.NextSubImage, action)
    }

    @Test
    fun diagonalVerticalDragPrefersSubImageNavigation() {
        val axis = resolveSwipeAxis(
            dragX = 60f,
            dragY = -72f,
            touchSlop = 18f,
            hasSubImages = true
        )
        val action = resolveFitModeSwipe(
            axis = axis,
            dragX = 60f,
            dragY = -90f,
            velocityX = 0f,
            velocityY = 0f,
            horizontalDistanceThreshold = 120f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = true
        )

        assertEquals(SwipeAxis.Vertical, axis)
        assertEquals(FitModeSwipeAction.NextSubImage, action)
    }

    @Test
    fun slowShortVerticalDragSettlesBack() {
        val action = resolveFitModeSwipe(
            axis = SwipeAxis.Vertical,
            dragX = 8f,
            dragY = 36f,
            velocityX = 0f,
            velocityY = 300f,
            horizontalDistanceThreshold = 120f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = true
        )

        assertEquals(FitModeSwipeAction.None, action)
    }

    @Test
    fun horizontalDragStillNavigatesFiles() {
        val axis = resolveSwipeAxis(
            dragX = -100f,
            dragY = 28f,
            touchSlop = 18f,
            hasSubImages = true
        )
        val action = resolveFitModeSwipe(
            axis = axis,
            dragX = -130f,
            dragY = 32f,
            velocityX = 0f,
            velocityY = 0f,
            horizontalDistanceThreshold = 120f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = true
        )

        assertEquals(SwipeAxis.Horizontal, axis)
        assertEquals(FitModeSwipeAction.NextFile, action)
    }

    @Test
    fun shortFastLeftFlickOpensNextFile() {
        val action = resolveFitModeSwipe(
            axis = SwipeAxis.Horizontal,
            dragX = -42f,
            dragY = 10f,
            velocityX = -1_800f,
            velocityY = 100f,
            horizontalDistanceThreshold = 80f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = false
        )

        assertEquals(FitModeSwipeAction.NextFile, action)
    }

    @Test
    fun slowShortHorizontalDragSettlesBack() {
        val action = resolveFitModeSwipe(
            axis = SwipeAxis.Horizontal,
            dragX = 36f,
            dragY = 8f,
            velocityX = 300f,
            velocityY = 0f,
            horizontalDistanceThreshold = 80f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = false
        )

        assertEquals(FitModeSwipeAction.None, action)
    }

    @Test
    fun diagonalVerticalDragOnRegularImageDoesNotNavigateFiles() {
        val axis = resolveSwipeAxis(
            dragX = 100f,
            dragY = -160f,
            touchSlop = 18f,
            hasSubImages = false
        )
        val action = resolveFitModeSwipe(
            axis = axis,
            dragX = 130f,
            dragY = -200f,
            velocityX = 0f,
            velocityY = 0f,
            horizontalDistanceThreshold = 120f,
            verticalDistanceThreshold = 80f,
            minimumFlingDistance = 24f,
            flingVelocityThreshold = 1_200f,
            hasSubImages = false
        )

        assertEquals(SwipeAxis.Undecided, axis)
        assertEquals(FitModeSwipeAction.None, action)
    }
}
