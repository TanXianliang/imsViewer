package com.imsviewer.android.ui

import kotlin.math.abs
import kotlin.math.max

internal enum class SwipeAxis {
    Undecided,
    Horizontal,
    Vertical
}

internal enum class FitModeSwipeAction {
    None,
    PreviousSubImage,
    NextSubImage,
    PreviousFile,
    NextFile
}

internal fun resolveSwipeAxis(
    dragX: Float,
    dragY: Float,
    touchSlop: Float,
    hasSubImages: Boolean
): SwipeAxis {
    val absX = abs(dragX)
    val absY = abs(dragY)
    if (max(absX, absY) < touchSlop) return SwipeAxis.Undecided

    // Give IMS navigation a forgiving vertical cone while retaining clear horizontal file swipes.
    return when {
        hasSubImages && absY >= absX * 0.75f -> SwipeAxis.Vertical
        absX >= absY * 1.15f -> SwipeAxis.Horizontal
        else -> SwipeAxis.Undecided
    }
}

internal fun resolveFitModeSwipe(
    axis: SwipeAxis,
    dragX: Float,
    dragY: Float,
    velocityX: Float,
    velocityY: Float,
    horizontalDistanceThreshold: Float,
    verticalDistanceThreshold: Float,
    minimumFlingDistance: Float,
    flingVelocityThreshold: Float,
    hasSubImages: Boolean
): FitModeSwipeAction {
    val (drag, velocity, distanceThreshold) = when (axis) {
        SwipeAxis.Horizontal -> Triple(dragX, velocityX, horizontalDistanceThreshold)
        SwipeAxis.Vertical -> Triple(dragY, velocityY, verticalDistanceThreshold)
        SwipeAxis.Undecided -> return FitModeSwipeAction.None
    }
    val passedDistance = abs(drag) >= distanceThreshold
    val passedVelocity = abs(drag) >= minimumFlingDistance &&
        abs(velocity) >= flingVelocityThreshold &&
        drag * velocity > 0f
    if (!passedDistance && !passedVelocity) return FitModeSwipeAction.None

    val direction = if (passedVelocity) velocity else drag
    return when (axis) {
        SwipeAxis.Horizontal -> if (direction < 0f) {
            FitModeSwipeAction.NextFile
        } else {
            FitModeSwipeAction.PreviousFile
        }
        SwipeAxis.Vertical -> if (!hasSubImages) {
            FitModeSwipeAction.None
        } else if (direction < 0f) {
            FitModeSwipeAction.NextSubImage
        } else {
            FitModeSwipeAction.PreviousSubImage
        }
        SwipeAxis.Undecided -> FitModeSwipeAction.None
    }
}
