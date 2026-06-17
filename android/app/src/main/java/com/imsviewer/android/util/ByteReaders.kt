package com.imsviewer.android.util

import java.nio.ByteBuffer
import java.nio.ByteOrder

internal fun ByteArray.u16Le(offset: Int): Int =
    ByteBuffer.wrap(this, offset, 2).order(ByteOrder.LITTLE_ENDIAN).short.toInt() and 0xffff

internal fun ByteArray.u32Le(offset: Int): Long =
    ByteBuffer.wrap(this, offset, 4).order(ByteOrder.LITTLE_ENDIAN).int.toLong() and 0xffffffffL
