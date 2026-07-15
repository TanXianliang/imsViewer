package com.imsviewer.android.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import com.imsviewer.android.domain.model.RemoteConnection
import com.imsviewer.android.domain.model.RemoteDownload
import com.imsviewer.android.domain.model.RemoteEntry
import com.imsviewer.android.util.u16Le
import com.imsviewer.android.util.u32Le
import com.jcraft.jsch.ChannelSftp
import com.jcraft.jsch.JSch
import com.jcraft.jsch.SftpProgressMonitor
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.util.Locale
import java.util.Properties
import java.util.Vector
import java.util.zip.CRC32

class SftpRepository(private val context: Context) {
    private val thumbnailBatchMutex = Mutex()

    suspend fun list(connection: RemoteConnection, path: String = connection.rootPath): List<RemoteEntry> =
        withContext(Dispatchers.IO) {
            withSftp(connection) { sftp ->
                @Suppress("UNCHECKED_CAST")
                val entries = sftp.ls(path) as Vector<ChannelSftp.LsEntry>
                entries
                    .asSequence()
                    .filter { it.filename != "." && it.filename != ".." }
                    .mapNotNull { entry ->
                        val childPath = joinRemotePath(path, entry.filename)
                        when {
                            entry.attrs.isDir -> RemoteEntry(
                                name = entry.filename,
                                path = childPath,
                                isDirectory = true,
                                sizeBytes = 0L,
                                modifiedTimeSeconds = entry.attrs.mTime.toLong()
                            )
                            isSupportedRemoteFile(entry.filename) -> RemoteEntry(
                                name = entry.filename,
                                path = childPath,
                                isDirectory = false,
                                sizeBytes = entry.attrs.size,
                                modifiedTimeSeconds = entry.attrs.mTime.toLong()
                            )
                            else -> null
                        }
                    }
                    .sortedWith(compareBy<RemoteEntry> { !it.isDirectory }.thenBy { it.name.lowercase(Locale.US) })
                    .toList()
            }
        }

    suspend fun download(connection: RemoteConnection, entry: RemoteEntry, maxCacheBytes: Long): RemoteDownload =
        withContext(Dispatchers.IO) {
            require(!entry.isDirectory) { "Remote entry is a directory" }
            val cacheDir = sftpCacheDir()
            cacheDir.mkdirs()
            val localFile = File(cacheDir, downloadCacheFileName(connection, entry))
            if (!isReusableDownload(localFile, entry)) {
                val temporaryFile = File.createTempFile("remote_download_", ".tmp", cacheDir)
                try {
                    withSftp(connection) { sftp ->
                        temporaryFile.outputStream().use { output ->
                            sftp.get(entry.path, output)
                        }
                    }
                    localFile.delete()
                    if (!temporaryFile.renameTo(localFile)) {
                        temporaryFile.copyTo(localFile, overwrite = true)
                    }
                } finally {
                    temporaryFile.delete()
                }
            }
            localFile.setLastModified(System.currentTimeMillis())
            trimDirectoryCache(cacheDir, maxCacheBytes, keepFile = localFile)
            RemoteDownload(
                uri = Uri.fromFile(localFile),
                fileName = entry.name,
                entry = entry
            )
        }

    suspend fun clearCache() = withContext(Dispatchers.IO) {
        clearFiles(sftpCacheDir())
        thumbnailBatchMutex.withLock {
            clearFiles(sftpThumbnailCacheDir())
        }
    }

    suspend fun makeThumbnails(
        connection: RemoteConnection,
        entries: List<RemoteEntry>,
        targetMaxSize: Int,
        onResult: (RemoteEntry, Result<Bitmap>) -> Unit
    ) =
        withContext(Dispatchers.IO) {
            thumbnailBatchMutex.withLock {
                val batchContext = currentCoroutineContext()
                val cacheDir = sftpThumbnailCacheDir()
                cacheDir.mkdirs()
                trimDirectoryCache(cacheDir, ThumbnailCacheLimitBytes, keepFile = null)

                val pendingEntries = mutableListOf<Pair<RemoteEntry, File>>()
                entries.forEach { entry ->
                    batchContext.ensureActive()
                    require(canMakeThumbnail(entry)) { "Remote thumbnail is not supported" }
                    val localFile = File(cacheDir, thumbnailFileName(connection, entry))
                    val cachedBitmap = loadCachedThumbnail(localFile, targetMaxSize)
                    if (cachedBitmap != null) {
                        onResult(entry, Result.success(cachedBitmap))
                    } else {
                        pendingEntries += entry to localFile
                    }
                }
                if (pendingEntries.isEmpty()) return@withLock

                var processedCount = 0
                try {
                    withSftp(connection) { sftp ->
                        pendingEntries.forEach { (entry, localFile) ->
                            batchContext.ensureActive()
                            val result = runCatching {
                                fetchAndCacheThumbnail(sftp, entry, localFile, targetMaxSize)
                            }
                            batchContext.ensureActive()
                            onResult(entry, result)
                            processedCount += 1
                        }
                    }
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Throwable) {
                    pendingEntries.drop(processedCount).forEach { (entry, _) ->
                        onResult(entry, Result.failure(error))
                    }
                }
            }
        }

    fun canMakeThumbnail(entry: RemoteEntry): Boolean {
        if (entry.isDirectory) return false
        return if (isImsFile(entry.name)) {
            entry.sizeBytes >= MinImsFileSize
        } else {
            entry.sizeBytes in 1..MaxRemoteThumbnailSourceBytes && isPlainImageFile(entry.name)
        }
    }

    private fun loadCachedThumbnail(file: File, targetMaxSize: Int): Bitmap? {
        if (!file.isFile || file.length() <= 0L) return null
        return runCatching { decodeThumbnail(file, targetMaxSize) }
            .onSuccess { file.setLastModified(System.currentTimeMillis()) }
            .getOrElse {
                file.delete()
                null
            }
    }

    private fun trimDirectoryCache(directory: File, maxCacheBytes: Long, keepFile: File?) {
        if (maxCacheBytes <= 0L) return
        val files = directory
            .listFiles()
            .orEmpty()
            .filter { it.isFile }
            .sortedByDescending { it.lastModified() }
        var totalBytes = keepFile?.takeIf { it.isFile }?.length() ?: 0L
        files.forEach { file ->
            if (file.absolutePath == keepFile?.absolutePath) return@forEach
            val fileBytes = file.length()
            if (totalBytes + fileBytes > maxCacheBytes) {
                file.delete()
            } else {
                totalBytes += fileBytes
            }
        }
    }

    private fun sftpCacheDir(): File = File(context.cacheDir, "sftp")

    private fun sftpThumbnailCacheDir(): File = File(context.cacheDir, "sftp_thumbnails")

    private fun clearFiles(directory: File) {
        directory.listFiles()?.forEach { file ->
            if (file.isFile) file.delete()
        }
    }

    private fun thumbnailFileName(connection: RemoteConnection, entry: RemoteEntry): String {
        return "${remoteEntryCacheKey(connection, entry)}.jpg"
    }

    private fun downloadCacheFileName(connection: RemoteConnection, entry: RemoteEntry): String {
        val safeName = entry.name
            .replace(Regex("[^A-Za-z0-9._-]"), "_")
            .takeLast(MaxCachedFileNameLength)
            .ifBlank { "remote_file" }
        return "${remoteEntryCacheKey(connection, entry)}_$safeName"
    }

    private fun remoteEntryCacheKey(connection: RemoteConnection, entry: RemoteEntry): String {
        val key = "${connection.username}@${connection.host}:${connection.port}:${entry.path}:${entry.sizeBytes}:${entry.modifiedTimeSeconds}"
        return java.security.MessageDigest.getInstance("SHA-256")
            .digest(key.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it) }
    }

    private fun isReusableDownload(file: File, entry: RemoteEntry): Boolean {
        if (!file.isFile || file.length() <= 0L) return false
        return entry.sizeBytes <= 0L || file.length() == entry.sizeBytes
    }

    private fun fetchAndCacheThumbnail(
        sftp: ChannelSftp,
        entry: RemoteEntry,
        thumbnailFile: File,
        targetMaxSize: Int
    ): Bitmap {
        val bitmap = if (isImsFile(entry.name)) {
            val sourceBytes = readImsThumbnail(sftp, entry)
            decodeThumbnail(sourceBytes, targetMaxSize)
        } else {
            val sourceFile = File.createTempFile("remote_thumb_", ".tmp", sftpThumbnailCacheDir())
            try {
                sourceFile.outputStream().use { output -> sftp.get(entry.path, output) }
                decodeThumbnail(sourceFile, targetMaxSize)
            } finally {
                sourceFile.delete()
            }
        }
        thumbnailFile.outputStream().use { output ->
            check(bitmap.compress(Bitmap.CompressFormat.JPEG, ThumbnailJpegQuality, output)) {
                "Cannot encode remote thumbnail"
            }
        }
        thumbnailFile.setLastModified(System.currentTimeMillis())
        trimDirectoryCache(sftpThumbnailCacheDir(), ThumbnailCacheLimitBytes, keepFile = thumbnailFile)
        return bitmap
    }

    private fun decodeThumbnail(file: File, targetMaxSize: Int): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        file.inputStream().use { BitmapFactory.decodeStream(it, null, bounds) }
        val options = BitmapFactory.Options().apply {
            inSampleSize = calculateInSampleSize(bounds.outWidth, bounds.outHeight, targetMaxSize)
        }
        return file.inputStream().use { stream ->
            BitmapFactory.decodeStream(stream, null, options)
        } ?: error("Cannot decode remote thumbnail")
    }

    private fun decodeThumbnail(bytes: ByteArray, targetMaxSize: Int): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        require(bounds.outWidth > 0 && bounds.outHeight > 0) { "Cannot read remote thumbnail bounds" }
        val options = BitmapFactory.Options().apply {
            inSampleSize = calculateInSampleSize(bounds.outWidth, bounds.outHeight, targetMaxSize)
        }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            ?: error("Cannot decode remote thumbnail")
    }

    private fun calculateInSampleSize(width: Int, height: Int, targetMaxSize: Int): Int {
        var sample = 1
        var currentWidth = width
        var currentHeight = height
        while (currentWidth / 2 >= targetMaxSize || currentHeight / 2 >= targetMaxSize) {
            sample *= 2
            currentWidth /= 2
            currentHeight /= 2
        }
        return sample
    }

    private fun readImsThumbnail(sftp: ChannelSftp, entry: RemoteEntry): ByteArray {
        val tailLength = if (entry.sizeBytes >= ImsTailSize) ImsTailSize else FooterSize.toLong()
        val tail = readRemoteRange(
            sftp = sftp,
            path = entry.path,
            offset = entry.sizeBytes - tailLength,
            length = tailLength
        )
        val indexOffset = tail.u32Le(tail.size - FooterSize)
        require(indexOffset < entry.sizeBytes - FooterSize) { "Invalid remote IMS index offset" }

        if (tail.size >= ImsTailSize.toInt() &&
            tail.copyOfRange(0, ThumbnailMagic.size).contentEquals(ThumbnailMagic)
        ) {
            return readEmbeddedImsThumbnail(sftp, entry, tail, indexOffset)
        }
        return readLegacyImsCover(sftp, entry, indexOffset)
    }

    private fun readEmbeddedImsThumbnail(
        sftp: ChannelSftp,
        entry: RemoteEntry,
        tail: ByteArray,
        indexOffset: Long
    ): ByteArray {
        val version = tail.u16Le(8)
        val format = tail[10].toInt() and 0xff
        val thumbnailOffset = tail.u32Le(12)
        val thumbnailLength = tail.u32Le(16)
        val width = tail.u16Le(20)
        val height = tail.u16Le(22)
        val expectedCrc32 = tail.u32Le(24)

        require(version == ThumbnailVersion) { "Unsupported IMS thumbnail version: $version" }
        require(format == ThumbnailFormatJpeg) { "Unsupported IMS thumbnail format: $format" }
        require(width > 0 && height > 0) { "Invalid IMS thumbnail dimensions" }
        require(thumbnailLength in 1L..MaxEmbeddedThumbnailBytes) { "Invalid IMS thumbnail length" }
        require(thumbnailOffset + thumbnailLength <= indexOffset) { "IMS thumbnail is out of bounds" }

        val thumbnailBytes = readRemoteRange(
            sftp = sftp,
            path = entry.path,
            offset = thumbnailOffset,
            length = thumbnailLength
        )
        val actualCrc32 = CRC32().apply { update(thumbnailBytes) }.value
        require(actualCrc32 == expectedCrc32) { "IMS thumbnail CRC32 mismatch" }
        return thumbnailBytes
    }

    private fun readLegacyImsCover(
        sftp: ChannelSftp,
        entry: RemoteEntry,
        indexOffset: Long
    ): ByteArray {
        val indexLength = entry.sizeBytes - FooterSize - indexOffset
        require(indexLength in 18L..MaxRemoteImsIndexBytes) { "Remote IMS index is too large" }
        val indexBytes = readRemoteRange(sftp, entry.path, indexOffset, indexLength)
        val coverRange = parseImsCoverRange(indexBytes, indexOffset)
        require(coverRange.length <= MaxRemoteThumbnailSourceBytes) {
            "Remote IMS cover is too large for thumbnail generation"
        }
        return readRemoteRange(sftp, entry.path, coverRange.offset, coverRange.length)
    }

    private fun parseImsCoverRange(indexBytes: ByteArray, indexOffset: Long): RemoteRange {
        require(indexBytes.size >= 18) { "Truncated remote IMS index" }
        val magic = indexBytes.copyOfRange(0, 8).toString(Charsets.US_ASCII)
        require(magic == ImsMagic) { "IMS magic not found" }
        val version = indexBytes.u16Le(8)
        require(version == 1 || version == 2) { "Unsupported IMS version: $version" }
        val imageCount = indexBytes.u32Le(10).toInt()
        require(imageCount in 1..MaxImsImageCount) { "Invalid IMS image count: $imageCount" }
        require(indexBytes.u32Le(14) == 0L) { "Unsupported IMS data start" }

        var cursor = 18
        var firstRange: RemoteRange? = null
        var coverRange: RemoteRange? = null
        repeat(imageCount) {
            require(cursor + 2 <= indexBytes.size) { "Truncated remote IMS entry" }
            val nameLength = indexBytes.u16Le(cursor)
            cursor += 2
            require(cursor + nameLength + 10 <= indexBytes.size) { "Invalid remote IMS entry" }
            cursor += nameLength
            cursor += 1
            val offset = indexBytes.u32Le(cursor)
            cursor += 4
            val length = indexBytes.u32Le(cursor)
            cursor += 4
            val isCover = (indexBytes[cursor].toInt() and 0xff) == 1
            cursor += 1
            require(length > 0 && offset + length <= indexOffset) { "Remote IMS image is out of bounds" }

            val range = RemoteRange(offset = offset, length = length)
            if (firstRange == null) firstRange = range
            if (isCover) coverRange = range

            if (version >= 2) {
                require(cursor + HashMetadataSize <= indexBytes.size) { "Truncated remote IMS hash" }
                val hashAlgorithm = indexBytes[cursor].toInt() and 0xff
                require(hashAlgorithm == HashAlgorithmSha256) {
                    "Unsupported IMS hash algorithm: $hashAlgorithm"
                }
                cursor += HashMetadataSize
            }
        }
        return coverRange ?: requireNotNull(firstRange) { "IMS cover not found" }
    }

    private fun readRemoteRange(
        sftp: ChannelSftp,
        path: String,
        offset: Long,
        length: Long
    ): ByteArray {
        require(offset >= 0L && length in 1..Int.MAX_VALUE.toLong()) { "Invalid remote read range" }
        val result = ByteArray(length.toInt())
        val input = sftp.get(path, null as SftpProgressMonitor?, offset)
        input.use { stream ->
            var totalRead = 0
            while (totalRead < result.size) {
                val bytesRead = stream.read(result, totalRead, result.size - totalRead)
                check(bytesRead > 0) { "Remote file ended before requested range" }
                totalRead += bytesRead
            }
        }
        return result
    }

    private fun <T> withSftp(connection: RemoteConnection, block: (ChannelSftp) -> T): T {
        val session = JSch().getSession(connection.username, connection.host, connection.port)
        session.setPassword(connection.password)
        session.setConfig(Properties().apply {
            put("StrictHostKeyChecking", "no")
        })
        session.connect(ConnectTimeoutMs)
        val channel = session.openChannel("sftp") as ChannelSftp
        return try {
            channel.connect(ConnectTimeoutMs)
            block(channel)
        } finally {
            channel.disconnect()
            session.disconnect()
        }
    }

    private fun joinRemotePath(parent: String, child: String): String {
        val normalizedParent = parent.ifBlank { "/" }
        return if (normalizedParent == "/") "/$child" else "${normalizedParent.trimEnd('/')}/$child"
    }

    private fun isSupportedRemoteFile(name: String): Boolean {
        val lower = name.lowercase(Locale.US)
        return lower.endsWith(".ims") ||
            lower.contains(".ims.") ||
            lower.endsWith(".jpg") ||
            lower.endsWith(".jpeg") ||
            lower.endsWith(".png") ||
            lower.endsWith(".webp") ||
            lower.endsWith(".avif")
    }

    private fun isPlainImageFile(name: String): Boolean {
        val lower = name.lowercase(Locale.US)
        return !lower.endsWith(".ims") &&
            !lower.contains(".ims.") &&
            (lower.endsWith(".jpg") ||
                lower.endsWith(".jpeg") ||
                lower.endsWith(".png") ||
                lower.endsWith(".webp") ||
                lower.endsWith(".avif"))
    }

    private fun isImsFile(name: String): Boolean {
        val lower = name.lowercase(Locale.US)
        return lower.endsWith(".ims") || lower.contains(".ims.")
    }

    private data class RemoteRange(val offset: Long, val length: Long)

    private companion object {
        const val ConnectTimeoutMs = 15_000
        const val MaxRemoteThumbnailSourceBytes = 20L * 1024L * 1024L
        const val ThumbnailCacheLimitBytes = 96L * 1024L * 1024L
        const val MaxEmbeddedThumbnailBytes = 1024L * 1024L
        const val MaxRemoteImsIndexBytes = 8L * 1024L * 1024L
        const val ThumbnailJpegQuality = 82
        const val MaxCachedFileNameLength = 96
        const val FooterSize = 4
        const val ImsTailSize = 36L
        const val MinImsFileSize = 22L
        const val ThumbnailVersion = 1
        const val ThumbnailFormatJpeg = 0
        const val ImsMagic = "IMSIMAGE"
        const val MaxImsImageCount = 4096
        const val HashMetadataSize = 33
        const val HashAlgorithmSha256 = 1
        val ThumbnailMagic = byteArrayOf(73, 77, 83, 84, 72, 77, 66, 0)
    }
}
