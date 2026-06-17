package com.imsviewer.android.domain.model

enum class ImageFormat(val label: String, val mimeType: String) {
    Jpeg("JPEG", "image/jpeg"),
    Png("PNG", "image/png"),
    Webp("WebP", "image/webp"),
    Avif("AVIF", "image/avif"),
    Ims("IMS", "application/octet-stream"),
    Unknown("Unknown", "application/octet-stream")
}
