package com.imsviewer.android.domain

import com.imsviewer.android.domain.model.ImageFormat
import com.imsviewer.android.domain.model.ImsFile
import com.imsviewer.android.domain.model.SubImage
import com.imsviewer.android.util.u16Le
import com.imsviewer.android.util.u32Le
import java.nio.charset.StandardCharsets

class ImsParser {
    fun parse(bytes: ByteArray): ImsFile {
        require(bytes.size >= 22) { "File is too small to be an IMS container" }

        val indexOffset = bytes.u32Le(bytes.size - 4)
        require(indexOffset >= 0 && indexOffset < bytes.size - 18) { "Invalid IMS index offset" }

        var cursor = indexOffset.toInt()
        val magic = bytes.copyOfRange(cursor, cursor + 8).toString(StandardCharsets.US_ASCII)
        require(magic == "IMSIMAGE") { "IMS magic not found" }
        cursor += 8

        val version = bytes.u16Le(cursor)
        cursor += 2
        require(version == 1) { "Unsupported IMS version: $version" }

        val imageCount = bytes.u32Le(cursor).toInt()
        cursor += 4
        require(imageCount > 0 && imageCount <= 4096) { "Invalid IMS image count: $imageCount" }

        val dataStart = bytes.u32Le(cursor)
        cursor += 4
        require(dataStart == 0L) { "Unsupported IMS data start: $dataStart" }

        val entries = buildList {
            repeat(imageCount) { index ->
                require(cursor + 2 <= bytes.size) { "Truncated IMS entry" }
                val nameLength = bytes.u16Le(cursor)
                cursor += 2
                require(nameLength >= 0 && cursor + nameLength + 10 <= bytes.size) {
                    "Invalid IMS entry name length"
                }

                val name = bytes.copyOfRange(cursor, cursor + nameLength).toString(StandardCharsets.UTF_8)
                cursor += nameLength

                val format = when (bytes[cursor].toInt() and 0xff) {
                    0 -> ImageFormat.Jpeg
                    1 -> ImageFormat.Png
                    else -> ImageFormat.Unknown
                }
                cursor += 1

                val offset = bytes.u32Le(cursor)
                cursor += 4
                val length = bytes.u32Le(cursor)
                cursor += 4
                val isCover = (bytes[cursor].toInt() and 0xff) == 1
                cursor += 1

                require(offset >= 0 && length > 0 && offset + length <= indexOffset) {
                    "IMS entry out of bounds: $name"
                }

                add(
                    SubImage(
                        index = index,
                        name = name,
                        format = format,
                        offset = offset,
                        length = length,
                        isCover = isCover
                    )
                )
            }
        }

        return ImsFile(version = version, entries = entries)
    }
}
