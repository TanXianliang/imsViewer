package com.imsviewer.android.viewmodel

import android.app.Application
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.imsviewer.android.data.ImageRepository
import com.imsviewer.android.domain.model.OpenRecord
import com.imsviewer.android.domain.model.OpenRecordKind
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
    private val recordsStore = application.getSharedPreferences("open_records", Context.MODE_PRIVATE)
    private val _uiState = MutableStateFlow(ViewerUiState())
    val uiState: StateFlow<ViewerUiState> = _uiState.asStateFlow()

    private var loadJob: Job? = null
    private var thumbnailJob: Job? = null
    private var folderThumbnailJob: Job? = null

    init {
        _uiState.update {
            it.copy(
                fileRecords = loadRecords(OpenRecordKind.File),
                folderRecords = loadRecords(OpenRecordKind.Folder),
                networkRecords = loadRecords(OpenRecordKind.Network)
            )
        }
    }

    fun openUri(uri: Uri, flags: Int = 0) {
        persistReadPermission(uri, flags)
        addUriRecord(OpenRecordKind.File, uri)
        openUriInternal(uri, refreshSequence = true)
    }

    fun openFolder(uri: Uri, flags: Int = 0) {
        persistReadPermission(uri, flags)
        addUriRecord(OpenRecordKind.Folder, uri)
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            _uiState.update {
                it.copy(screenState = ScreenState.Loading, errorMessage = null, loadingMessage = "Opening folder")
            }
            runCatching { repository.scanTreeImages(uri) }
                .onSuccess { sequence ->
                    if (sequence.isEmpty()) {
                        _uiState.update {
                            it.copy(
                                screenState = ScreenState.Error,
                                errorMessage = "No supported IMS or image files in this folder"
                            )
                        }
                    } else {
                        _uiState.update {
                            it.copy(
                                screenState = ScreenState.Folder,
                                folderTitle = repository.displayName(uri),
                                folderSequence = sequence,
                                folderThumbnails = emptyMap(),
                                failedFolderThumbnails = emptySet(),
                                fileSequence = sequence,
                                fileIndex = 0,
                                isHomeVisible = false,
                                isInfoSheetVisible = false,
                                isSwitchingFile = false,
                                errorMessage = null
                            )
                        }
                        loadFolderThumbnails(sequence)
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

    fun openUriSequence(uris: List<Uri>, flags: Int = 0) {
        val sequence = uris.distinct()
        val firstUri = sequence.firstOrNull() ?: return
        sequence.forEach { persistReadPermission(it, flags) }
        addUriRecord(OpenRecordKind.File, firstUri)
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            openLoadedUriFromSequence(firstUri, sequence, 0, showFullScreenLoading = true)
        }
    }

    fun openRecord(record: OpenRecord) {
        val uri = record.uri?.let(Uri::parse) ?: return
        when (record.kind) {
            OpenRecordKind.File -> {
                persistReadPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                openUriInternal(uri, refreshSequence = true, sourceRecord = record)
            }
            OpenRecordKind.Folder -> openFolderFromRecord(record, uri)
            OpenRecordKind.Network -> Unit
        }
    }

    private fun openFolderFromRecord(record: OpenRecord, uri: Uri) {
        persistReadPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            _uiState.update { it.copy(screenState = ScreenState.Loading, errorMessage = null, loadingMessage = "Opening folder") }
            runCatching { repository.scanTreeImages(uri) }
                .onSuccess { sequence ->
                    if (sequence.isEmpty()) {
                        showRecordOpenError(record, "No supported IMS or image files in this folder")
                    } else {
                        _uiState.update {
                            it.copy(
                                screenState = ScreenState.Folder,
                                folderTitle = record.title,
                                folderSequence = sequence,
                                folderThumbnails = emptyMap(),
                                failedFolderThumbnails = emptySet(),
                                fileSequence = sequence,
                                fileIndex = 0,
                                isHomeVisible = false,
                                isInfoSheetVisible = false,
                                isSwitchingFile = false,
                                errorMessage = null
                            )
                        }
                        loadFolderThumbnails(sequence)
                    }
                }
                .onFailure { error ->
                    showRecordOpenError(record, error.message ?: "Failed to open folder")
                }
        }
    }

    fun openFolderImage(index: Int) {
        val state = _uiState.value
        val uri = state.folderSequence.getOrNull(index) ?: return
        openUriFromSequence(uri, state.folderSequence, index)
    }

    fun showHome() {
        folderThumbnailJob?.cancel()
        _uiState.update {
            it.copy(
                screenState = ScreenState.Empty,
                currentImage = null,
                fileSequence = emptyList(),
                fileIndex = 0,
                folderTitle = null,
                folderSequence = emptyList(),
                folderThumbnails = emptyMap(),
                failedFolderThumbnails = emptySet(),
                thumbnails = emptyMap(),
                isHomeVisible = true,
                isUiVisible = true,
                isInfoSheetVisible = false,
                isSwitchingFile = false,
                errorMessage = null
            )
        }
    }

    fun showFolder() {
        val state = _uiState.value
        if (state.folderSequence.isEmpty()) {
            showHome()
            return
        }
        _uiState.update {
            it.copy(
                screenState = ScreenState.Folder,
                currentImage = null,
                thumbnails = emptyMap(),
                isUiVisible = true,
                isInfoSheetVisible = false,
                isSwitchingFile = false,
                errorMessage = null
            )
        }
    }

    fun addNetworkRecord(host: String, path: String) {
        val normalizedHost = host.trim()
        val normalizedPath = path.trim().ifBlank { "/" }
        if (normalizedHost.isBlank()) return
        val record = OpenRecord(
            id = "network:$normalizedHost:$normalizedPath",
            kind = OpenRecordKind.Network,
            title = normalizedHost,
            subtitle = normalizedPath,
            host = normalizedHost,
            path = normalizedPath
        )
        upsertRecord(record)
    }

    fun updateNetworkRecord(record: OpenRecord, host: String, path: String) {
        val normalizedHost = host.trim()
        val normalizedPath = path.trim().ifBlank { "/" }
        if (normalizedHost.isBlank()) return
        val updated = record.copy(
            id = "network:$normalizedHost:$normalizedPath",
            title = normalizedHost,
            subtitle = normalizedPath,
            host = normalizedHost,
            path = normalizedPath,
            uri = null
        )
        val records = (listOf(updated.copy(updatedAt = System.currentTimeMillis())) +
            loadRecords(OpenRecordKind.Network).filterNot { it.id == record.id || it.id == updated.id })
            .take(MaxRecords)
        saveRecords(OpenRecordKind.Network, records)
        publishRecords(OpenRecordKind.Network, records)
    }

    fun deleteRecord(record: OpenRecord) {
        val records = loadRecords(record.kind).filterNot { it.id == record.id }
        saveRecords(record.kind, records)
        publishRecords(record.kind, records)
    }

    fun clearRecords(kind: OpenRecordKind) {
        saveRecords(kind, emptyList())
        publishRecords(kind, emptyList())
    }

    private fun openUriInternal(uri: Uri, refreshSequence: Boolean, sourceRecord: OpenRecord? = null) {
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
            val normalizedSequence = sequence.ifEmpty { listOf(uri) }
            val currentIndex = repository.indexInSequence(uri, normalizedSequence).takeIf { it >= 0 } ?: 0
            val targetUri = normalizedSequence.getOrNull(currentIndex) ?: uri

            runCatching { repository.open(targetUri) }
                .onSuccess { image ->
                    _uiState.update {
                        it.copy(
                            screenState = ScreenState.Ready,
                            currentImage = image,
                            fileSequence = normalizedSequence,
                            fileIndex = currentIndex,
                            thumbnails = emptyMap(),
                            isHomeVisible = false,
                            isUiVisible = true,
                            isInfoSheetVisible = false,
                            errorMessage = null
                        )
                    }
                    loadThumbnails(image)
                }
                .onFailure { error ->
                    val message = error.message ?: "Failed to open file"
                    if (sourceRecord != null) {
                        showRecordOpenError(sourceRecord, message)
                    } else {
                        _uiState.update {
                            it.copy(
                                screenState = ScreenState.Error,
                                errorMessage = message
                            )
                        }
                    }
                }
        }
    }

    private fun openUriFromSequence(uri: Uri, sequence: List<Uri>, index: Int) {
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            openLoadedUriFromSequence(uri, sequence, index, showFullScreenLoading = false)
        }
    }

    private suspend fun openLoadedUriFromSequence(
        uri: Uri,
        sequence: List<Uri>,
        index: Int,
        showFullScreenLoading: Boolean
    ) {
        _uiState.update {
            if (showFullScreenLoading) {
                it.copy(
                    screenState = ScreenState.Loading,
                    isSwitchingFile = false,
                    errorMessage = null,
                    loadingMessage = "Opening image"
                )
            } else {
                it.copy(isSwitchingFile = true, errorMessage = null, loadingMessage = "Opening image")
            }
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
                        isHomeVisible = false,
                        isUiVisible = if (showFullScreenLoading) true else it.isUiVisible,
                        isInfoSheetVisible = false,
                        isSwitchingFile = false,
                        errorMessage = null
                    )
                }
                loadThumbnails(image)
            }
            .onFailure { error ->
                _uiState.update {
                    val message = error.message ?: "Failed to open file"
                    if (showFullScreenLoading) {
                        it.copy(
                            screenState = ScreenState.Error,
                            isSwitchingFile = false,
                            errorMessage = message
                        )
                    } else {
                        it.copy(
                            isSwitchingFile = false,
                            errorMessage = message
                        )
                    }
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

    private fun addUriRecord(kind: OpenRecordKind, uri: Uri) {
        val title = runCatching { repository.displayName(uri) }.getOrDefault(uri.lastPathSegment ?: "Untitled")
        val record = OpenRecord(
            id = "${kind.name}:${uri}",
            kind = kind,
            title = title,
            subtitle = uri.toString(),
            uri = uri.toString()
        )
        upsertRecord(record)
    }

    private fun upsertRecord(record: OpenRecord) {
        val records = loadRecords(record.kind)
            .filterNot { it.id == record.id }
            .let { listOf(record.copy(updatedAt = System.currentTimeMillis())) + it }
            .take(MaxRecords)
        saveRecords(record.kind, records)
        publishRecords(record.kind, records)
    }

    private fun publishRecords(kind: OpenRecordKind, records: List<OpenRecord>) {
        _uiState.update {
            when (kind) {
                OpenRecordKind.File -> it.copy(fileRecords = records)
                OpenRecordKind.Folder -> it.copy(folderRecords = records)
                OpenRecordKind.Network -> it.copy(networkRecords = records)
            }
        }
    }

    private fun loadRecords(kind: OpenRecordKind): List<OpenRecord> {
        return recordsStore.getStringSet(kind.preferenceKey, emptySet()).orEmpty()
            .mapNotNull { decodeRecord(kind, it) }
            .sortedByDescending { it.updatedAt }
    }

    private fun showRecordOpenError(record: OpenRecord, message: String) {
        _uiState.update {
            it.copy(
                screenState = ScreenState.Empty,
                currentImage = null,
                isHomeVisible = true,
                isSwitchingFile = false,
                errorMessage = "${record.title}: $message"
            )
        }
    }

    private fun saveRecords(kind: OpenRecordKind, records: List<OpenRecord>) {
        recordsStore.edit()
            .putStringSet(kind.preferenceKey, records.map(::encodeRecord).toSet())
            .apply()
    }

    private fun encodeRecord(record: OpenRecord): String {
        return listOf(
            record.id,
            record.title,
            record.subtitle,
            record.uri.orEmpty(),
            record.host.orEmpty(),
            record.path.orEmpty(),
            record.updatedAt.toString()
        ).joinToString(separator = "\u001F") { it.replace("\u001F", " ") }
    }

    private fun decodeRecord(kind: OpenRecordKind, value: String): OpenRecord? {
        val parts = value.split("\u001F")
        if (parts.size < 7) return null
        return OpenRecord(
            id = parts[0],
            kind = kind,
            title = parts[1],
            subtitle = parts[2],
            uri = parts[3].ifBlank { null },
            host = parts[4].ifBlank { null },
            path = parts[5].ifBlank { null },
            updatedAt = parts[6].toLongOrNull() ?: 0L
        )
    }

    private val OpenRecordKind.preferenceKey: String
        get() = when (this) {
            OpenRecordKind.File -> "files"
            OpenRecordKind.Folder -> "folders"
            OpenRecordKind.Network -> "networks"
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

    private fun loadFolderThumbnails(sequence: List<Uri>) {
        folderThumbnailJob?.cancel()
        folderThumbnailJob = viewModelScope.launch {
            sequence.forEach { uri ->
                val state = _uiState.value
                if (state.folderThumbnails.containsKey(uri) || state.failedFolderThumbnails.contains(uri)) return@forEach
                runCatching { repository.makeThumbnail(uri) }
                    .onSuccess { bitmap ->
                        _uiState.update { state ->
                            state.copy(folderThumbnails = trimFolderThumbnailCache(state.folderThumbnails + (uri to bitmap)))
                        }
                    }
                    .onFailure {
                        _uiState.update { state ->
                            state.copy(failedFolderThumbnails = state.failedFolderThumbnails + uri)
                        }
                    }
            }
        }
    }

    private fun trimFolderThumbnailCache(cache: Map<Uri, android.graphics.Bitmap>): Map<Uri, android.graphics.Bitmap> {
        if (cache.size <= MaxFolderThumbnails) return cache
        return cache.entries.toList().takeLast(MaxFolderThumbnails).associate { it.toPair() }
    }

    private companion object {
        const val MaxRecords = 30
        const val MaxFolderThumbnails = 160
    }
}
