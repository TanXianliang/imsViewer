package com.imsviewer.android.domain

internal fun wrappedAdjacentIndex(
    currentIndex: Int,
    itemCount: Int,
    direction: Int
): Int? {
    if (itemCount <= 1 || currentIndex !in 0 until itemCount || direction == 0) return null
    return ((currentIndex + direction) % itemCount + itemCount) % itemCount
}
