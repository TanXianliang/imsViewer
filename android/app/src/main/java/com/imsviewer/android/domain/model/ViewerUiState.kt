package com.imsviewer.android.domain.model

import android.graphics.Bitmap
import android.net.Uri

enum class ScreenState {
    Empty,
    Folder,
    Loading,
    Ready,
    Error
}

data class ViewerUiState(
    val screenState: ScreenState = ScreenState.Empty,
    val currentImage: ViewerImage? = null,
    val fileSequence: List<Uri> = emptyList(),
    val fileIndex: Int = 0,
    val fileRecords: List<OpenRecord> = emptyList(),
    val folderRecords: List<OpenRecord> = emptyList(),
    val networkRecords: List<OpenRecord> = emptyList(),
    val folderTitle: String? = null,
    val folderSequence: List<Uri> = emptyList(),
    val folderThumbnails: Map<Uri, Bitmap> = emptyMap(),
    val isHomeVisible: Boolean = true,
    val thumbnails: Map<Int, Bitmap> = emptyMap(),
    val isUiVisible: Boolean = true,
    val isInfoSheetVisible: Boolean = false,
    val isSwitchingFile: Boolean = false,
    val errorMessage: String? = null,
    val loadingMessage: String = "Loading"
)
