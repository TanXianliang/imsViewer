package com.imsviewer.android.domain.model

import android.graphics.Bitmap
import android.net.Uri

data class ViewerImage(
    val bitmap: Bitmap,
    val fileName: String,
    val uri: Uri,
    val format: ImageFormat,
    val sizeBytes: Long,
    val width: Int,
    val height: Int,
    val imsFile: ImsFile? = null,
    val subImageIndex: Int = 0
) {
    val subImageCount: Int
        get() = imsFile?.entries?.size ?: 1

    val currentSubImage: SubImage?
        get() = imsFile?.entries?.getOrNull(subImageIndex)
}
