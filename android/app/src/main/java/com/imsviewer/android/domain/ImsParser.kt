package com.imsviewer.android.domain

import com.imsviewer.android.domain.model.ImageFormat
import com.imsviewer.android.domain.model.ImsFile
import com.imsviewer.android.domain.model.ImsThumbnail
import com.imsviewer.android.domain.model.SubImage
import com.imsviewer.android.util.u16Le
import com.imsviewer.android.util.u32Le
import java.nio.charset.StandardCharsets
import java.util.zip.CRC32

class ImsParser {
    fun parse(bytes: ByteArray): ImsFile {
        require(bytes.size >= 22) { "File is too small to be an IMS container" }

        val footerOffset = bytes.size - FooterSize
        val indexOffset = bytes.u32Le(footerOffset)
        require(indexOffset <= Int.MAX_VALUE && indexOffset < bytes.size - 18L) {
            "Invalid IMS index offset"
        }
        val thumbnail = parseThumbnail(bytes, indexOffset)
        val metadataEnd = if (thumbnail == null) footerOffset else footerOffset - ThumbnailTrailerSize
        val imageDataEnd = thumbnail?.offset ?: indexOffset

        var cursor = indexOffset.toInt()
        require(cursor + 18 <= metadataEnd) { "Truncated IMS header" }
        val magic = bytes.copyOfRange(cursor, cursor + 8).toString(StandardCharsets.US_ASCII)
        require(magic == "IMSIMAGE") { "IMS magic not found" }
        cursor += 8

        val version = bytes.u16Le(cursor)
        cursor += 2
        require(version == 1 || version == 2) { "Unsupported IMS version: $version" }

        val imageCount = bytes.u32Le(cursor).toInt()
        cursor += 4
        require(imageCount > 0 && imageCount <= 4096) { "Invalid IMS image count: $imageCount" }

        val dataStart = bytes.u32Le(cursor)
        cursor += 4
        require(dataStart == 0L) { "Unsupported IMS data start: $dataStart" }

        val entries = buildList {
            repeat(imageCount) { index ->
                require(cursor + 2 <= metadataEnd) { "Truncated IMS entry" }
                val nameLength = bytes.u16Le(cursor)
                cursor += 2
                require(cursor + nameLength + 10 <= metadataEnd) {
                    "Invalid IMS entry name length"
                }

                val name = bytes.copyOfRange(cursor, cursor + nameLength).toString(StandardCharsets.UTF_8)
                cursor += nameLength

                val format = when (bytes[cursor].toInt() and 0xff) {
                    0 -> ImageFormat.Jpeg
                    1 -> ImageFormat.Png
                    2 -> ImageFormat.Webp
                    3 -> ImageFormat.Avif
                    else -> ImageFormat.Unknown
                }
                cursor += 1

                val offset = bytes.u32Le(cursor)
                cursor += 4
                val length = bytes.u32Le(cursor)
                cursor += 4
                val isCover = (bytes[cursor].toInt() and 0xff) == 1
                cursor += 1

                require(offset >= 0 && length > 0 && offset + length <= imageDataEnd) {
                    "IMS entry out of bounds: $name"
                }

                if (version >= 2) {
                    require(cursor + HashMetadataSize <= metadataEnd) { "Truncated IMS hash: $name" }
                    val hashAlgorithm = bytes[cursor].toInt() and 0xff
                    require(hashAlgorithm == HashAlgorithmSha256) {
                        "Unsupported IMS hash algorithm: $hashAlgorithm"
                    }
                    cursor += HashMetadataSize
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

        return ImsFile(version = version, entries = entries, thumbnail = thumbnail)
    }

    private fun parseThumbnail(bytes: ByteArray, indexOffset: Long): ImsThumbnail? {
        if (bytes.size < FooterSize + ThumbnailTrailerSize) return null
        val trailerOffset = bytes.size - FooterSize - ThumbnailTrailerSize
        val magic = bytes.copyOfRange(trailerOffset, trailerOffset + 8)
            .toString(StandardCharsets.US_ASCII)
        if (magic != ThumbnailMagic) return null

        val version = bytes.u16Le(trailerOffset + 8)
        val formatValue = bytes[trailerOffset + 10].toInt() and 0xff
        val flags = bytes[trailerOffset + 11].toInt() and 0xff
        val offset = bytes.u32Le(trailerOffset + 12)
        val length = bytes.u32Le(trailerOffset + 16)
        val width = bytes.u16Le(trailerOffset + 20)
        val height = bytes.u16Le(trailerOffset + 22)
        val expectedCrc32 = bytes.u32Le(trailerOffset + 24)

        require(version == 1) { "Unsupported IMS thumbnail version: $version" }
        require(formatValue == 0) { "Unsupported IMS thumbnail format: $formatValue" }
        require(length > 0 && offset + length <= indexOffset) { "IMS thumbnail is out of bounds" }
        require(offset <= Int.MAX_VALUE && length <= Int.MAX_VALUE) { "IMS thumbnail is too large" }
        require(width > 0 && height > 0) { "Invalid IMS thumbnail dimensions" }

        val actualCrc32 = CRC32().apply {
            update(bytes, offset.toInt(), length.toInt())
        }.value
        require(actualCrc32 == expectedCrc32) { "IMS thumbnail CRC32 mismatch" }

        return ImsThumbnail(
            version = version,
            format = ImageFormat.Jpeg,
            flags = flags,
            offset = offset,
            length = length,
            width = width,
            height = height,
            crc32 = expectedCrc32
        )
    }

    private companion object {
        const val FooterSize = 4
        const val ThumbnailTrailerSize = 32
        const val ThumbnailMagic = "IMSTHMB\u0000"
        const val HashMetadataSize = 33
        const val HashAlgorithmSha256 = 1
    }
}
