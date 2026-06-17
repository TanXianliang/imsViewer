package com.imsviewer.android.domain

import android.content.ContentResolver
import android.net.Uri
import com.imsviewer.android.domain.model.ImageFormat
import java.util.Locale

class ImageFormatDetector {
    fun detect(contentResolver: ContentResolver, uri: Uri, fileName: String): ImageFormat {
        val lowerName = fileName.lowercase(Locale.US)
        if (lowerName.endsWith(".ims") || lowerName.contains(".ims.")) return ImageFormat.Ims

        return when {
            lowerName.endsWith(".jpg") || lowerName.endsWith(".jpeg") -> ImageFormat.Jpeg
            lowerName.endsWith(".png") -> ImageFormat.Png
            lowerName.endsWith(".webp") -> ImageFormat.Webp
            lowerName.endsWith(".avif") -> ImageFormat.Avif
            else -> when (contentResolver.getType(uri)?.lowercase(Locale.US)) {
                "image/jpeg" -> ImageFormat.Jpeg
                "image/png" -> ImageFormat.Png
                "image/webp" -> ImageFormat.Webp
                "image/avif" -> ImageFormat.Avif
                else -> ImageFormat.Unknown
            }
        }
    }
}
