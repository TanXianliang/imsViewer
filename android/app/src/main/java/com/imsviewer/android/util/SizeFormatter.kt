package com.imsviewer.android.util

import java.util.Locale
import kotlin.math.log10
import kotlin.math.pow

object SizeFormatter {
    private val units = arrayOf("B", "KB", "MB", "GB")

    fun format(bytes: Long): String {
        if (bytes <= 0) return "0 B"
        val digitGroups = (log10(bytes.toDouble()) / log10(1024.0)).toInt().coerceIn(0, units.lastIndex)
        val value = bytes / 1024.0.pow(digitGroups.toDouble())
        return String.format(Locale.US, "%.1f %s", value, units[digitGroups])
    }
}
