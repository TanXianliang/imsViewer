package com.imsviewer.android.domain.model

data class ImsFile(
    val version: Int,
    val entries: List<SubImage>,
    val thumbnail: ImsThumbnail? = null
)

data class ImsThumbnail(
    val version: Int,
    val format: ImageFormat,
    val flags: Int,
    val offset: Long,
    val length: Long,
    val width: Int,
    val height: Int,
    val crc32: Long
)

data class SubImage(
    val index: Int,
    val name: String,
    val format: ImageFormat,
    val offset: Long,
    val length: Long,
    val isCover: Boolean,
    val width: Int = 0,
    val height: Int = 0
)
