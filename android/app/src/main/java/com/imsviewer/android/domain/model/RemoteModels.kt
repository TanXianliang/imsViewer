package com.imsviewer.android.domain.model

import android.net.Uri

data class RemoteConnection(
    val host: String,
    val port: Int = 22,
    val username: String,
    val password: String,
    val rootPath: String = "/"
)

data class RemoteEntry(
    val name: String,
    val path: String,
    val isDirectory: Boolean,
    val sizeBytes: Long = 0L,
    val modifiedTimeSeconds: Long = 0L
)

data class RemoteDownload(
    val uri: Uri,
    val fileName: String,
    val entry: RemoteEntry
)
