package com.imsviewer.android.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import com.imsviewer.android.domain.model.RemoteConnection
import com.imsviewer.android.domain.model.RemoteDownload
import com.imsviewer.android.domain.model.RemoteEntry
import com.jcraft.jsch.ChannelSftp
import com.jcraft.jsch.JSch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.util.Locale
import java.util.Properties
import java.util.Vector

class SftpRepository(private val context: Context) {
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
            trimCache(maxCacheBytes, keepFile = localFile)
            RemoteDownload(
                uri = Uri.fromFile(localFile),
                fileName = entry.name,
                entry = entry
            )
        }

    suspend fun clearCache() = withContext(Dispatchers.IO) {
        clearFiles(sftpCacheDir())
        clearFiles(sftpThumbnailCacheDir())
    }

    suspend fun makeThumbnail(connection: RemoteConnection, entry: RemoteEntry, targetMaxSize: Int): Bitmap =
        withContext(Dispatchers.IO) {
            require(canMakeThumbnail(entry)) { "Remote thumbnail is not supported" }
            val cacheDir = sftpThumbnailCacheDir()
            cacheDir.mkdirs()
            val localFile = File(cacheDir, thumbnailFileName(connection, entry))
            if (localFile.exists() && localFile.length() > 0L) {
                return@withContext runCatching { decodeThumbnail(localFile, targetMaxSize) }
                    .getOrElse {
                        localFile.delete()
                        fetchAndCacheThumbnail(connection, entry, localFile, targetMaxSize)
                    }
            }
            fetchAndCacheThumbnail(connection, entry, localFile, targetMaxSize)
        }

    fun canMakeThumbnail(entry: RemoteEntry): Boolean {
        return !entry.isDirectory &&
            entry.sizeBytes in 1..MaxRemoteThumbnailSourceBytes &&
            isPlainImageFile(entry.name)
    }

    private fun trimCache(maxCacheBytes: Long, keepFile: File) {
        if (maxCacheBytes <= 0L) return
        val files = sftpCacheDir()
            .listFiles()
            .orEmpty()
            .filter { it.isFile }
            .sortedByDescending { it.lastModified() }
        var totalBytes = keepFile.length()
        files.forEach { file ->
            if (file.absolutePath == keepFile.absolutePath) return@forEach
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
        connection: RemoteConnection,
        entry: RemoteEntry,
        thumbnailFile: File,
        targetMaxSize: Int
    ): Bitmap {
        val sourceFile = File.createTempFile("remote_thumb_", ".tmp", sftpThumbnailCacheDir())
        return try {
            withSftp(connection) { sftp ->
                sourceFile.outputStream().use { output ->
                    sftp.get(entry.path, output)
                }
            }
            val bitmap = decodeThumbnail(sourceFile, targetMaxSize)
            thumbnailFile.outputStream().use { output ->
                check(bitmap.compress(Bitmap.CompressFormat.JPEG, ThumbnailJpegQuality, output)) {
                    "Cannot encode remote thumbnail"
                }
            }
            bitmap
        } finally {
            sourceFile.delete()
        }
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

    private companion object {
        const val ConnectTimeoutMs = 15_000
        const val MaxRemoteThumbnailSourceBytes = 20L * 1024L * 1024L
        const val ThumbnailJpegQuality = 82
        const val MaxCachedFileNameLength = 96
    }
}
