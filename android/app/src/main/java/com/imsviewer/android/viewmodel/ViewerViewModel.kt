package com.imsviewer.android.viewmodel

import android.app.Application
import android.content.Intent
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.imsviewer.android.data.ImageRepository
import com.imsviewer.android.domain.model.ScreenState
import com.imsviewer.android.domain.model.ViewerUiState
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

class ViewerViewModel(application: Application) : AndroidViewModel(application) {
    private val repository = ImageRepository(application)
    private val _uiState = MutableStateFlow(ViewerUiState())
    val uiState: StateFlow<ViewerUiState> = _uiState.asStateFlow()

    private var loadJob: Job? = null
    private var thumbnailJob: Job? = null

    fun openUri(uri: Uri, flags: Int = 0) {
        persistReadPermission(uri, flags)
        openUriInternal(uri, refreshSequence = true)
    }

    fun openFolder(uri: Uri, flags: Int = 0) {
        persistReadPermission(uri, flags)
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            _uiState.update {
                it.copy(screenState = ScreenState.Loading, errorMessage = null, loadingMessage = "Opening folder")
            }
            runCatching { repository.scanTreeImages(uri) }
                .onSuccess { sequence ->
                    val firstUri = sequence.firstOrNull()
                    if (firstUri == null) {
                        _uiState.update {
                            it.copy(
                                screenState = ScreenState.Error,
                                errorMessage = "No supported IMS or image files in this folder"
                            )
                        }
                    } else {
                        openLoadedUriFromSequence(firstUri, sequence, 0)
                    }
                }
                .onFailure { error ->
                    _uiState.update {
                        it.copy(
                            screenState = ScreenState.Error,
                            errorMessage = error.message ?: "Failed to open folder"
                        )
                    }
                }
        }
    }

    private fun openUriInternal(uri: Uri, refreshSequence: Boolean) {
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            _uiState.update {
                it.copy(screenState = ScreenState.Loading, errorMessage = null, loadingMessage = "Opening image")
            }
            val sequence = if (refreshSequence) {
                repository.scanSiblingImages(uri)
            } else {
                _uiState.value.fileSequence.ifEmpty { listOf(uri) }
            }
            runCatching { repository.open(uri) }
                .onSuccess { image ->
                    val normalizedSequence = sequence.ifEmpty { listOf(uri) }
                    val currentIndex = normalizedSequence.indexOf(uri).takeIf { it >= 0 } ?: 0
                    _uiState.update {
                        it.copy(
                            screenState = ScreenState.Ready,
                            currentImage = image,
                            fileSequence = normalizedSequence,
                            fileIndex = currentIndex,
                            thumbnails = emptyMap(),
                            isUiVisible = true,
                            isInfoSheetVisible = false,
                            errorMessage = null
                        )
                    }
                    loadThumbnails(image)
                }
                .onFailure { error ->
                    _uiState.update {
                        it.copy(
                            screenState = ScreenState.Error,
                            errorMessage = error.message ?: "Failed to open file"
                        )
                    }
                }
        }
    }

    private fun openUriFromSequence(uri: Uri, sequence: List<Uri>, index: Int) {
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            openLoadedUriFromSequence(uri, sequence, index)
        }
    }

    private suspend fun openLoadedUriFromSequence(uri: Uri, sequence: List<Uri>, index: Int) {
        _uiState.update {
            it.copy(screenState = ScreenState.Loading, errorMessage = null, loadingMessage = "Opening image")
        }
        runCatching { repository.open(uri) }
            .onSuccess { image ->
                _uiState.update {
                    it.copy(
                        screenState = ScreenState.Ready,
                        currentImage = image,
                        fileSequence = sequence,
                        fileIndex = index,
                        thumbnails = emptyMap(),
                        isUiVisible = true,
                        isInfoSheetVisible = false,
                        errorMessage = null
                    )
                }
                loadThumbnails(image)
            }
            .onFailure { error ->
                _uiState.update {
                    it.copy(
                        screenState = ScreenState.Error,
                        errorMessage = error.message ?: "Failed to open file"
                    )
                }
            }
    }

    fun openSubImage(index: Int) {
        val current = _uiState.value.currentImage ?: return
        if (current.imsFile == null || index !in current.imsFile.entries.indices || index == current.subImageIndex) {
            return
        }

        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            _uiState.update { it.copy(loadingMessage = "Switching image") }
            runCatching { repository.openSubImage(current, index) }
                .onSuccess { image ->
                    _uiState.update {
                        it.copy(screenState = ScreenState.Ready, currentImage = image, errorMessage = null)
                    }
                    loadThumbnails(image)
                }
                .onFailure { error ->
                    _uiState.update {
                        it.copy(errorMessage = error.message ?: "Failed to switch sub image")
                    }
                }
        }
    }

    fun openNextSubImage() {
        val current = _uiState.value.currentImage ?: return
        if (current.subImageCount <= 1) return
        openSubImage((current.subImageIndex + 1) % current.subImageCount)
    }

    fun openPreviousSubImage() {
        val current = _uiState.value.currentImage ?: return
        if (current.subImageCount <= 1) return
        openSubImage((current.subImageIndex - 1 + current.subImageCount) % current.subImageCount)
    }

    fun openNextFile() {
        val state = _uiState.value
        if (state.fileSequence.size <= 1) return
        val nextIndex = (state.fileIndex + 1).coerceAtMost(state.fileSequence.lastIndex)
        if (nextIndex != state.fileIndex) openUriFromSequence(state.fileSequence[nextIndex], state.fileSequence, nextIndex)
    }

    fun openPreviousFile() {
        val state = _uiState.value
        if (state.fileSequence.size <= 1) return
        val previousIndex = (state.fileIndex - 1).coerceAtLeast(0)
        if (previousIndex != state.fileIndex) {
            openUriFromSequence(state.fileSequence[previousIndex], state.fileSequence, previousIndex)
        }
    }

    fun toggleChrome() {
        _uiState.update { it.copy(isUiVisible = !it.isUiVisible) }
    }

    fun showInfoSheet() {
        _uiState.update { it.copy(isInfoSheetVisible = true, isUiVisible = true) }
    }

    fun hideInfoSheet() {
        _uiState.update { it.copy(isInfoSheetVisible = false) }
    }

    fun consumeError() {
        _uiState.update { it.copy(errorMessage = null) }
    }

    fun shareIntent(): Intent? {
        val image = _uiState.value.currentImage ?: return null
        return Intent(Intent.ACTION_SEND).apply {
            type = if (image.format.mimeType.startsWith("image/")) image.format.mimeType else "*/*"
            putExtra(Intent.EXTRA_STREAM, image.uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
    }

    private fun persistReadPermission(uri: Uri, flags: Int) {
        if (flags and Intent.FLAG_GRANT_READ_URI_PERMISSION == 0) return
        runCatching {
            getApplication<Application>().contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        }
    }

    private fun loadThumbnails(image: com.imsviewer.android.domain.model.ViewerImage) {
        val entries = image.imsFile?.entries.orEmpty()
        if (entries.size <= 1) return

        thumbnailJob?.cancel()
        thumbnailJob = viewModelScope.launch {
            entries.forEach { entry ->
                if (_uiState.value.thumbnails.containsKey(entry.index)) return@forEach
                runCatching { repository.makeThumbnail(image, entry) }
                    .onSuccess { bitmap ->
                        _uiState.update { state ->
                            state.copy(thumbnails = state.thumbnails + (entry.index to bitmap))
                        }
                    }
            }
        }
    }
}
