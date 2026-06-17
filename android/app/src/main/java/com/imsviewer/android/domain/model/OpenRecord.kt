package com.imsviewer.android.domain.model

enum class OpenRecordKind {
    File,
    Folder,
    Network
}

data class OpenRecord(
    val id: String,
    val kind: OpenRecordKind,
    val title: String,
    val subtitle: String,
    val uri: String? = null,
    val host: String? = null,
    val path: String? = null,
    val updatedAt: Long = System.currentTimeMillis()
)
