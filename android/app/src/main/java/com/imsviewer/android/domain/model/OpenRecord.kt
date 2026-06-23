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
    val alias: String? = null,
    val host: String? = null,
    val port: Int = 22,
    val username: String? = null,
    val password: String? = null,
    val hasSavedPassword: Boolean = false,
    val path: String? = null,
    val updatedAt: Long = System.currentTimeMillis()
)
