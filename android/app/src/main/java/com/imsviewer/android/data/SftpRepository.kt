package com.imsviewer.android.data

import android.content.Context
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
                                sizeBytes = 0L
                            )
                            isSupportedRemoteFile(entry.filename) -> RemoteEntry(
                                name = entry.filename,
                                path = childPath,
                                isDirectory = false,
                                sizeBytes = entry.attrs.size
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
            val safeName = entry.name.replace(Regex("[^A-Za-z0-9._-]"), "_")
            val localFile = File(cacheDir, "${System.currentTimeMillis()}_$safeName")
            withSftp(connection) { sftp ->
                localFile.outputStream().use { output ->
                    sftp.get(entry.path, output)
                }
            }
            trimCache(maxCacheBytes, keepFile = localFile)
            RemoteDownload(
                uri = Uri.fromFile(localFile),
                fileName = entry.name,
                entry = entry
            )
        }

    suspend fun clearCache() = withContext(Dispatchers.IO) {
        sftpCacheDir().listFiles()?.forEach { file ->
            if (file.isFile) file.delete()
        }
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
            totalBytes += file.length()
            if (totalBytes > maxCacheBytes) {
                file.delete()
            }
        }
    }

    private fun sftpCacheDir(): File = File(context.cacheDir, "sftp")

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

    private companion object {
        const val ConnectTimeoutMs = 15_000
    }
}
