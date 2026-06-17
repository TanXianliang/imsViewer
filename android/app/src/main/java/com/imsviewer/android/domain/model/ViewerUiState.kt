package com.imsviewer.android.domain.model

import android.graphics.Bitmap
import android.net.Uri

enum class ScreenState {
    Empty,
    Loading,
    Ready,
    Error
}

data class ViewerUiState(
    val screenState: ScreenState = ScreenState.Empty,
    val currentImage: ViewerImage? = null,
    val fileSequence: List<Uri> = emptyList(),
    val fileIndex: Int = 0,
    val thumbnails: Map<Int, Bitmap> = emptyMap(),
    val isUiVisible: Boolean = true,
    val isInfoSheetVisible: Boolean = false,
    val errorMessage: String? = null,
    val loadingMessage: String = "Loading"
)
