package com.imsviewer.android.viewmodel

import android.app.Application
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.imsviewer.android.data.ImageRepository
import com.imsviewer.android.data.SecurePasswordStore
import com.imsviewer.android.data.SftpRepository
import com.imsviewer.android.domain.wrappedAdjacentIndex
import com.imsviewer.android.domain.model.OpenRecord
import com.imsviewer.android.domain.model.OpenRecordKind
import com.imsviewer.android.domain.model.RemoteConnection
import com.imsviewer.android.domain.model.RemoteEntry
import com.imsviewer.android.domain.model.ScreenState
import com.imsviewer.android.domain.model.ViewerUiState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.PriorityQueue
import java.util.UUID

class ViewerViewModel(application: Application) : AndroidViewModel(application) {
    private val repository = ImageRepository(application)
    private val sftpRepository = SftpRepository(application)
    private val recordsStore = application.getSharedPreferences("open_records", Context.MODE_PRIVATE)
    private val settingsStore = application.getSharedPreferences("viewer_settings", Context.MODE_PRIVATE)
    private val passwordStore = SecurePasswordStore(application)
    private val _uiState = MutableStateFlow(ViewerUiState())
    val uiState: StateFlow<ViewerUiState> = _uiState.asStateFlow()

    private var loadJob: Job? = null
    private var thumbnailJob: Job? = null
    private var folderThumbnailJob: Job? = null
    private var adjacentFilePreviewJob: Job? = null
    private var remoteThumbnailJob: Job? = null
    private var remoteVisibleRange: IntRange? = null
    private val sessionPasswords = mutableMapOf<String, String>()

    init {
        val fileRecords = loadRecords(OpenRecordKind.File)
        val folderRecords = loadRecords(OpenRecordKind.Folder)
        val networkRecords = loadRecords(OpenRecordKind.Network)
        saveRecords(OpenRecordKind.Network, networkRecords)
        _uiState.update {
            it.copy(
                fileRecords = fileRecords,
                folderRecords = folderRecords,
                networkRecords = networkRecords,
                networkCacheLimitMb = loadNetworkCacheLimitMb()
            )
        }
    }

    fun openUri(uri: Uri, flags: Int = 0) {
        setHomeTab(OpenRecordKind.File)
        persistReadPermission(uri, flags)
        addUriRecord(OpenRecordKind.File, uri)
        openUriInternal(uri, refreshSequence = true)
    }

    fun openFolder(uri: Uri, flags: Int = 0) {
        setHomeTab(OpenRecordKind.Folder)
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
                                homeTab = OpenRecordKind.Folder,
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
        setHomeTab(OpenRecordKind.File)
        sequence.forEach { persistReadPermission(it, flags) }
        addUriRecord(OpenRecordKind.File, firstUri)
        loadJob?.cancel()
        loadJob = viewModelScope.launch {
            openLoadedUriFromSequence(firstUri, sequence, 0, showFullScreenLoading = true)
        }
    }

    fun openRecord(record: OpenRecord) {
        setHomeTab(record.kind)
        when (record.kind) {
            OpenRecordKind.File -> {
                val uri = record.uri?.let(Uri::parse) ?: return
                persistReadPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                openUriInternal(uri, refreshSequence = true, sourceRecord = record)
            }
            OpenRecordKind.Folder -> {
                val uri = record.uri?.let(Uri::parse) ?: return
                openFolderFromRecord(record, uri)
            }
            OpenRecordKind.Network -> openNetworkRecord(record)
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
                                homeTab = OpenRecordKind.Folder,
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
        loadJob?.cancel()
        folderThumbnailJob?.cancel()
        adjacentFilePreviewJob?.cancel()
        remoteThumbnailJob?.cancel()
        remoteVisibleRange = null
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
                remoteConnection = null,
                remoteTitle = null,
                remotePath = "/",
                remoteEntries = emptyList(),
                remoteFileSequence = emptyList(),
                remoteFileIndex = -1,
                remoteThumbnails = emptyMap(),
                failedRemoteThumbnails = emptySet(),
                remoteBackStack = emptyList(),
                isRemoteBusy = false,
                passwordPromptRecord = null,
                thumbnails = emptyMap(),
                isHomeVisible = true,
                isUiVisible = true,
                isInfoSheetVisible = false,
                isSwitchingFile = false,
                isSwitchingSubImage = false,
                errorMessage = null
            )
        }
    }

    fun selectHomeTab(kind: OpenRecordKind) {
        setHomeTab(kind)
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
                homeTab = OpenRecordKind.Folder,
                currentImage = null,
                thumbnails = emptyMap(),
                isUiVisible = true,
                isInfoSheetVisible = false,
                isSwitchingFile = false,
                isSwitchingSubImage = false,
                errorMessage = null
            )
        }
    }

    fun showRemote() {
        val state = _uiState.value
        if (state.remoteConnection == null) {
            showHome()
            return
        }
        remoteThumbnailJob?.cancel()
        remoteVisibleRange = null
        _uiState.update {
            it.copy(
                screenState = ScreenState.Remote,
                homeTab = OpenRecordKind.Network,
                currentImage = null,
                thumbnails = emptyMap(),
                isUiVisible = true,
                isInfoSheetVisible = false,
                isSwitchingFile = false,
                isSwitchingSubImage = false,
                errorMessage = null
            )
        }
    }

    fun updateRemoteVisibleRange(visibleRange: IntRange) {
        val state = _uiState.value
        val connection = state.remoteConnection ?: return
        if (state.screenState != ScreenState.Remote || state.remoteEntries.isEmpty()) return
        val firstIndex = visibleRange.first.coerceIn(state.remoteEntries.indices)
        val lastIndex = visibleRange.last.coerceIn(state.remoteEntries.indices)
        if (firstIndex > lastIndex) return
        val normalizedRange = firstIndex..lastIndex
        if (normalizedRange == remoteVisibleRange && remoteThumbnailJob?.isActive == true) return
        remoteVisibleRange = normalizedRange
        loadRemoteThumbnails(connection, state.remoteEntries, normalizedRange)
    }

    fun addNetworkRecord(
        alias: String,
        host: String,
        port: String,
        username: String,
        password: String,
        savePassword: Boolean,
        path: String
    ) {
        val normalizedHost = host.trim()
        val normalizedUser = username.trim()
        val normalizedPort = port.toIntOrNull()?.coerceIn(1, 65535) ?: 22
        val normalizedPath = path.trim().ifBlank { "/" }
        val normalizedAlias = alias.trim()
        if (normalizedHost.isBlank() || normalizedUser.isBlank()) return
        val title = normalizedAlias.ifBlank { "$normalizedHost:$normalizedPort" }
        val recordId = newNetworkRecordId()
        val hasSavedPassword = persistNetworkPassword(recordId, password, savePassword)
        val record = OpenRecord(
            id = recordId,
            kind = OpenRecordKind.Network,
            title = title,
            subtitle = normalizedPath,
            alias = normalizedAlias.ifBlank { null },
            host = normalizedHost,
            port = normalizedPort,
            username = normalizedUser,
            hasSavedPassword = hasSavedPassword,
            path = normalizedPath
        )
        upsertRecord(record)
    }

    fun updateNetworkRecord(
        record: OpenRecord,
        alias: String,
        host: String,
        port: String,
        username: String,
        password: String,
        savePassword: Boolean,
        path: String
    ) {
        val normalizedHost = host.trim()
        val normalizedUser = username.trim()
        val normalizedPort = port.toIntOrNull()?.coerceIn(1, 65535) ?: 22
        val normalizedPath = path.trim().ifBlank { "/" }
        val normalizedAlias = alias.trim()
        if (normalizedHost.isBlank() || normalizedUser.isBlank()) return
        val title = normalizedAlias.ifBlank { "$normalizedHost:$normalizedPort" }
        val existingPassword = if (savePassword && password.isBlank()) {
            passwordStore.load(record.id) ?: sessionPasswords[record.id]
        } else {
            null
        }
        val hasSavedPassword = persistNetworkPassword(
            record.id,
            password.ifBlank { existingPassword.orEmpty() },
            savePassword
        )
        val updated = record.copy(
            title = title,
            subtitle = normalizedPath,
            alias = normalizedAlias.ifBlank { null },
            host = normalizedHost,
            port = normalizedPort,
            username = normalizedUser,
            password = null,
            hasSavedPassword = hasSavedPassword,
            path = normalizedPath,
            uri = null
        )
        if (!savePassword) {
            passwordStore.delete(record.id)
            sessionPasswords.remove(record.id)
        }
        val records = (listOf(updated.copy(updatedAt = System.currentTimeMillis())) +
            loadRecords(OpenRecordKind.Network).filterNot { it.id == record.id })
            .take(MaxRecords)
        saveRecords(OpenRecordKind.Network, records)
        publishRecords(OpenRecordKind.Network, records)
    }

    fun submitNetworkPassword(record: OpenRecord, password: String, savePassword: Boolean) {
        sessionPasswords[record.id] = password
        if (savePassword) {
            setNetworkRecordSavedPassword(record.id, persistNetworkPassword(record.id, password, savePassword = true))
        }
        _uiState.update { it.copy(passwordPromptRecord = null) }
        openNetworkRecord(record)
    }

    fun dismissNetworkPasswordPrompt() {
        _uiState.update { it.copy(passwordPromptRecord = null) }
    }

    fun openRemoteEntry(entry: RemoteEntry) {
        val state = _uiState.value
        val connection = state.remoteConnection ?: return
        if (entry.isDirectory) {
            loadRemoteDirectory(connection, entry.path, pushCurrentPath = true)
        } else {
            val sequence = state.remoteFileSequence.ifEmpty { state.remoteEntries.filterNot { it.isDirectory } }
            val index = sequence.indexOfFirst { it.path == entry.path }.takeIf { it >= 0 } ?: return
            downloadRemoteFile(
                connection = connection,
                entry = entry,
                sequence = sequence,
                index = index,
                showFullScreenLoading = true
            )
        }
    }

    fun navigateRemoteBack() {
        val state = _uiState.value
        val connection = state.remoteConnection ?: return
        val previousPath = state.remoteBackStack.lastOrNull()
        if (previousPath == null) {
            showHome()
        } else {
            loadRemoteDirectory(connection, previousPath, popBackStack = true)
        }
    }

    private fun openNetworkRecord(record: OpenRecord) {
        setHomeTab(OpenRecordKind.Network)
        val password = sessionPasswords[record.id] ?: passwordStore.load(record.id)
        if (password == null) {
            _uiState.update { it.copy(passwordPromptRecord = record, errorMessage = null) }
            return
        }
        val connection = record.toRemoteConnection(password) ?: run {
            showRecordOpenError(record, "Missing SFTP connection fields")
            return
        }
        loadRemoteDirectory(connection, connection.rootPath, title = record.title)
    }

    private fun loadRemoteDirectory(
        connection: RemoteConnection,
        path: String,
        title: String = _uiState.value.remoteTitle ?: "${connection.username}@${connection.host}:${connection.port}",
        pushCurrentPath: Boolean = false,
        popBackStack: Boolean = false
    ) {
        loadJob?.cancel()
        remoteThumbnailJob?.cancel()
        remoteVisibleRange = null
        val previousState = _uiState.value
        loadJob = viewModelScope.launch {
            _uiState.update {
                it.copy(
                    screenState = ScreenState.Remote,
                    homeTab = OpenRecordKind.Network,
                    remoteConnection = connection,
                    remoteTitle = title,
                    remotePath = path,
                    remoteFileSequence = emptyList(),
                    remoteFileIndex = -1,
                    remoteThumbnails = emptyMap(),
                    failedRemoteThumbnails = emptySet(),
                    isRemoteBusy = true,
                    isHomeVisible = false,
                    errorMessage = null
                )
            }
            runCatching { sftpRepository.list(connection, path) }
                .onSuccess { entries ->
                    _uiState.update {
                        val nextBackStack = when {
                            pushCurrentPath -> it.remoteBackStack + previousState.remotePath
                            popBackStack -> it.remoteBackStack.dropLast(1)
                            else -> it.remoteBackStack
                        }
                        it.copy(
                            screenState = ScreenState.Remote,
                            homeTab = OpenRecordKind.Network,
                            remoteEntries = entries,
                            remoteFileSequence = entries.filterNot { entry -> entry.isDirectory },
                            remoteFileIndex = -1,
                            remotePath = path,
                            remoteBackStack = nextBackStack,
                            remoteThumbnails = emptyMap(),
                            failedRemoteThumbnails = emptySet(),
                            isRemoteBusy = false,
                            errorMessage = null
                        )
                    }
                }
                .onFailure { error ->
                    _uiState.update {
                        it.copy(
                            screenState = ScreenState.Remote,
                            isRemoteBusy = false,
                            errorMessage = error.message ?: "Failed to list remote directory"
                        )
                    }
                }
        }
    }

    private fun downloadRemoteFile(
        connection: RemoteConnection,
        entry: RemoteEntry,
        sequence: List<RemoteEntry>,
        index: Int,
        showFullScreenLoading: Boolean
    ) {
        loadJob?.cancel()
        remoteThumbnailJob?.cancel()
        loadJob = viewModelScope.launch {
            _uiState.update {
                if (showFullScreenLoading) {
                    it.copy(isRemoteBusy = true, errorMessage = null, loadingMessage = "Downloading ${entry.name}")
                } else {
                    it.copy(isSwitchingFile = true, errorMessage = null, loadingMessage = "Downloading ${entry.name}")
                }
            }
            runCatching { sftpRepository.download(connection, entry, networkCacheLimitBytes()) }
                .onSuccess { download ->
                    _uiState.update {
                        it.copy(
                            remoteFileSequence = sequence,
                            isRemoteBusy = false
                        )
                    }
                    openLoadedUriFromSequence(
                        uri = download.uri,
                        sequence = listOf(download.uri),
                        index = 0,
                        showFullScreenLoading = showFullScreenLoading,
                        remoteIndex = index
                    )
                }
                .onFailure { error ->
                    _uiState.update {
                        if (showFullScreenLoading) {
                            it.copy(
                                screenState = ScreenState.Remote,
                                remoteFileSequence = sequence,
                                remoteFileIndex = -1,
                                isRemoteBusy = false,
                                errorMessage = error.message ?: "Failed to download remote file"
                            )
                        } else {
                            it.copy(
                                isSwitchingFile = false,
                                errorMessage = error.message ?: "Failed to download remote file"
                            )
                        }
                    }
                }
        }
    }

    fun updateNetworkCacheLimit(limitMb: String) {
        val normalizedLimit = limitMb.toIntOrNull()?.coerceIn(MinNetworkCacheLimitMb, MaxNetworkCacheLimitMb)
            ?: DefaultNetworkCacheLimitMb
        settingsStore.edit().putInt(NetworkCacheLimitMbKey, normalizedLimit).apply()
        _uiState.update { it.copy(networkCacheLimitMb = normalizedLimit) }
    }

    fun clearNetworkCache() {
        viewModelScope.launch {
            runCatching { sftpRepository.clearCache() }
                .onSuccess {
                    _uiState.update { it.copy(errorMessage = "Network cache cleared") }
                }
                .onFailure { error ->
                    _uiState.update { it.copy(errorMessage = error.message ?: "Failed to clear network cache") }
                }
        }
    }

    fun deleteRecord(record: OpenRecord) {
        if (record.kind == OpenRecordKind.Network) {
            passwordStore.delete(record.id)
            sessionPasswords.remove(record.id)
        }
        val records = loadRecords(record.kind).filterNot { it.id == record.id }
        saveRecords(record.kind, records)
        publishRecords(record.kind, records)
    }

    fun clearRecords(kind: OpenRecordKind) {
        if (kind == OpenRecordKind.Network) {
            loadRecords(kind).forEach { record ->
                passwordStore.delete(record.id)
                sessionPasswords.remove(record.id)
            }
        }
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
                            homeTab = sourceRecord?.kind ?: OpenRecordKind.File,
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
                    loadAdjacentFilePreviews(normalizedSequence, currentIndex)
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
        showFullScreenLoading: Boolean,
        remoteIndex: Int? = null
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
                    val homeTab = when {
                        it.remoteConnection != null -> OpenRecordKind.Network
                        it.folderSequence.isNotEmpty() -> OpenRecordKind.Folder
                        else -> OpenRecordKind.File
                    }
                    it.copy(
                        screenState = ScreenState.Ready,
                        homeTab = homeTab,
                        currentImage = image,
                        fileSequence = sequence,
                        fileIndex = index,
                        remoteFileIndex = remoteIndex ?: it.remoteFileIndex,
                        thumbnails = emptyMap(),
                        isHomeVisible = false,
                        isUiVisible = if (showFullScreenLoading) true else it.isUiVisible,
                        isInfoSheetVisible = false,
                        isSwitchingFile = false,
                        errorMessage = null
                    )
                }
                loadThumbnails(image)
                if (remoteIndex == null) {
                    loadAdjacentFilePreviews(sequence, index)
                }
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
        val state = _uiState.value
        val current = state.currentImage ?: return
        if (state.isSwitchingFile || state.isSwitchingSubImage) return
        if (current.imsFile == null || index !in current.imsFile.entries.indices || index == current.subImageIndex) {
            return
        }

        loadJob?.cancel()
        _uiState.update {
            it.copy(isSwitchingSubImage = true, loadingMessage = "Switching image", errorMessage = null)
        }
        loadJob = viewModelScope.launch {
            try {
                val image = repository.openSubImage(current, index)
                _uiState.update {
                    it.copy(
                        screenState = ScreenState.Ready,
                        currentImage = image,
                        isSwitchingSubImage = false,
                        errorMessage = null
                    )
                }
                loadThumbnails(image)
            } catch (error: CancellationException) {
                _uiState.update { it.copy(isSwitchingSubImage = false) }
                throw error
            } catch (error: Exception) {
                _uiState.update {
                    it.copy(
                        isSwitchingSubImage = false,
                        errorMessage = error.message ?: "Failed to switch sub image"
                    )
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
        if (openAdjacentRemoteFile(state, direction = 1)) return
        if (state.isSwitchingFile || state.isSwitchingSubImage) return
        val nextIndex = wrappedAdjacentIndex(state.fileIndex, state.fileSequence.size, 1) ?: return
        openUriFromSequence(state.fileSequence[nextIndex], state.fileSequence, nextIndex)
    }

    fun openPreviousFile() {
        val state = _uiState.value
        if (openAdjacentRemoteFile(state, direction = -1)) return
        if (state.isSwitchingFile || state.isSwitchingSubImage) return
        val previousIndex = wrappedAdjacentIndex(state.fileIndex, state.fileSequence.size, -1) ?: return
        openUriFromSequence(state.fileSequence[previousIndex], state.fileSequence, previousIndex)
    }

    private fun openAdjacentRemoteFile(state: ViewerUiState, direction: Int): Boolean {
        val connection = state.remoteConnection ?: return false
        val sequence = state.remoteFileSequence
        if (sequence.isEmpty() || state.remoteFileIndex !in sequence.indices) return false
        if (state.isSwitchingFile || state.isSwitchingSubImage || state.isRemoteBusy) return true
        val targetIndex = wrappedAdjacentIndex(state.remoteFileIndex, sequence.size, direction) ?: return true
        downloadRemoteFile(
            connection = connection,
            entry = sequence[targetIndex],
            sequence = sequence,
            index = targetIndex,
            showFullScreenLoading = false
        )
        return true
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

    private fun setHomeTab(kind: OpenRecordKind) {
        _uiState.update { it.copy(homeTab = kind) }
    }

    private fun loadNetworkCacheLimitMb(): Int {
        return settingsStore.getInt(NetworkCacheLimitMbKey, DefaultNetworkCacheLimitMb)
            .coerceIn(MinNetworkCacheLimitMb, MaxNetworkCacheLimitMb)
    }

    private fun networkCacheLimitBytes(): Long = _uiState.value.networkCacheLimitMb.toLong() * BytesPerMb

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
            record.alias.orEmpty(),
            record.host.orEmpty(),
            record.port.toString(),
            record.username.orEmpty(),
            if (record.hasSavedPassword) SavedPasswordMarker else "",
            record.path.orEmpty(),
            record.updatedAt.toString()
        ).joinToString(separator = "\u001F") { it.replace("\u001F", " ") }
    }

    private fun decodeRecord(kind: OpenRecordKind, value: String): OpenRecord? {
        val parts = value.split("\u001F")
        if (parts.size < 7) return null
        if (parts.size == 7) {
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
        if (parts.size == 10) {
            val password = parts[7].ifBlank { null }
            val recordId = parts[0]
            val migratedPassword = password != null && !passwordStore.has(recordId) && passwordStore.save(recordId, password)
            return OpenRecord(
                id = parts[0],
                kind = kind,
                title = parts[1],
                subtitle = parts[2],
                uri = parts[3].ifBlank { null },
                host = parts[4].ifBlank { null },
                port = parts[5].toIntOrNull() ?: 22,
                username = parts[6].ifBlank { null },
                password = null,
                hasSavedPassword = migratedPassword || passwordStore.has(recordId),
                path = parts[8].ifBlank { null },
                updatedAt = parts[9].toLongOrNull() ?: 0L
            )
        }
        if (parts.size < 11) return null
        val recordId = parts[0]
        val passwordOrSavedFlag = parts[8]
        val legacyPassword = passwordOrSavedFlag
            .takeUnless { it == SavedPasswordMarker }
            ?.ifBlank { null }
        val migratedPassword = legacyPassword != null && !passwordStore.has(recordId) &&
            passwordStore.save(recordId, legacyPassword)
        return OpenRecord(
            id = parts[0],
            kind = kind,
            title = parts[1],
            subtitle = parts[2],
            uri = parts[3].ifBlank { null },
            alias = parts[4].ifBlank { null },
            host = parts[5].ifBlank { null },
            port = parts[6].toIntOrNull() ?: 22,
            username = parts[7].ifBlank { null },
            password = null,
            hasSavedPassword = passwordOrSavedFlag == SavedPasswordMarker || migratedPassword || passwordStore.has(recordId),
            path = parts[9].ifBlank { null },
            updatedAt = parts.getOrNull(10)?.toLongOrNull() ?: 0L
        )
    }

    private fun setNetworkRecordSavedPassword(recordId: String, hasSavedPassword: Boolean) {
        val records = loadRecords(OpenRecordKind.Network).map { record ->
            if (record.id == recordId) record.copy(hasSavedPassword = hasSavedPassword) else record
        }
        saveRecords(OpenRecordKind.Network, records)
        publishRecords(OpenRecordKind.Network, records)
    }

    private fun persistNetworkPassword(recordId: String, password: String, savePassword: Boolean): Boolean {
        if (!savePassword) {
            passwordStore.delete(recordId)
            sessionPasswords.remove(recordId)
            return false
        }
        if (password.isNotBlank()) {
            return passwordStore.save(recordId, password)
        }
        return passwordStore.has(recordId)
    }

    private fun newNetworkRecordId(): String = "network:${UUID.randomUUID()}"

    private fun OpenRecord.toRemoteConnection(password: String): RemoteConnection? {
        val host = host?.takeIf { it.isNotBlank() } ?: return null
        val username = username?.takeIf { it.isNotBlank() } ?: return null
        return RemoteConnection(
            host = host,
            port = port,
            username = username,
            password = password,
            rootPath = path?.takeIf { it.isNotBlank() } ?: "/"
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

        val adjacentEntries = listOf(
            entries[(image.subImageIndex + 1) % entries.size],
            entries[(image.subImageIndex - 1 + entries.size) % entries.size],
            entries[image.subImageIndex]
        )
        val prioritizedEntries = (adjacentEntries + entries).distinctBy { it.index }

        thumbnailJob?.cancel()
        thumbnailJob = viewModelScope.launch {
            prioritizedEntries.forEach { entry ->
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

    private fun loadAdjacentFilePreviews(sequence: List<Uri>, currentIndex: Int) {
        val previewUris = listOfNotNull(
            wrappedAdjacentIndex(currentIndex, sequence.size, -1)?.let(sequence::getOrNull),
            wrappedAdjacentIndex(currentIndex, sequence.size, 1)?.let(sequence::getOrNull)
        ).distinct()
        if (previewUris.isEmpty()) return

        adjacentFilePreviewJob?.cancel()
        adjacentFilePreviewJob = viewModelScope.launch {
            previewUris.forEach { uri ->
                val state = _uiState.value
                if (state.folderThumbnails.containsKey(uri) || state.failedFolderThumbnails.contains(uri)) {
                    return@forEach
                }
                try {
                    val bitmap = repository.makeThumbnail(uri)
                    _uiState.update { current ->
                        if (uri in current.fileSequence) {
                            current.copy(
                                folderThumbnails = trimFolderThumbnailCache(
                                    current.folderThumbnails + (uri to bitmap)
                                )
                            )
                        } else {
                            current
                        }
                    }
                } catch (error: CancellationException) {
                    throw error
                } catch (_: Exception) {
                    _uiState.update { current ->
                        if (uri in current.fileSequence) {
                            current.copy(failedFolderThumbnails = current.failedFolderThumbnails + uri)
                        } else {
                            current
                        }
                    }
                }
            }
        }
    }

    private fun loadRemoteThumbnails(
        connection: RemoteConnection,
        entries: List<RemoteEntry>,
        visibleRange: IntRange
    ) {
        remoteThumbnailJob?.cancel()
        val state = _uiState.value
        val firstPrefetchIndex = (visibleRange.first - RemoteThumbnailPrefetchCount).coerceAtLeast(0)
        val lastPrefetchIndex = (visibleRange.last + RemoteThumbnailPrefetchCount).coerceAtMost(entries.lastIndex)
        val queue = PriorityQueue(
            compareBy<RemoteThumbnailCandidate> { it.priority }
                .thenBy { it.index }
        )
        for (index in firstPrefetchIndex..lastPrefetchIndex) {
            val entry = entries[index]
            if (!sftpRepository.canMakeThumbnail(entry)) continue
            if (state.remoteThumbnails.containsKey(entry.path) || state.failedRemoteThumbnails.contains(entry.path)) {
                continue
            }
            val priority = when {
                index < visibleRange.first -> visibleRange.first - index
                index > visibleRange.last -> index - visibleRange.last
                else -> 0
            }
            queue.add(RemoteThumbnailCandidate(index = index, priority = priority, entry = entry))
        }
        val candidates = buildList {
            while (queue.isNotEmpty()) add(queue.remove().entry)
        }
        if (candidates.isEmpty()) return
        remoteThumbnailJob = viewModelScope.launch {
            try {
                sftpRepository.makeThumbnails(connection, candidates, RemoteThumbnailSizePx) { entry, result ->
                    result.onSuccess { bitmap ->
                        _uiState.update { current ->
                            if (current.remoteConnection == connection) {
                                current.copy(remoteThumbnails = trimRemoteThumbnailCache(current.remoteThumbnails + (entry.path to bitmap)))
                            } else {
                                current
                            }
                        }
                    }
                    result.onFailure {
                        _uiState.update { current ->
                            if (current.remoteConnection == connection) {
                                current.copy(failedRemoteThumbnails = current.failedRemoteThumbnails + entry.path)
                            } else {
                                current
                            }
                        }
                    }
                }
            } catch (error: CancellationException) {
                throw error
            }
        }
    }

    private fun trimFolderThumbnailCache(cache: Map<Uri, android.graphics.Bitmap>): Map<Uri, android.graphics.Bitmap> {
        if (cache.size <= MaxFolderThumbnails) return cache
        return cache.entries.toList().takeLast(MaxFolderThumbnails).associate { it.toPair() }
    }

    private fun trimRemoteThumbnailCache(cache: Map<String, android.graphics.Bitmap>): Map<String, android.graphics.Bitmap> {
        if (cache.size <= MaxRemoteThumbnails) return cache
        return cache.entries.toList().takeLast(MaxRemoteThumbnails).associate { it.toPair() }
    }

    private data class RemoteThumbnailCandidate(
        val index: Int,
        val priority: Int,
        val entry: RemoteEntry
    )

    private companion object {
        const val MaxRecords = 30
        const val MaxFolderThumbnails = 160
        const val MaxRemoteThumbnails = 160
        const val RemoteThumbnailSizePx = 180
        const val RemoteThumbnailPrefetchCount = 8
        const val SavedPasswordMarker = "__saved_password__"
        const val NetworkCacheLimitMbKey = "network_cache_limit_mb"
        const val DefaultNetworkCacheLimitMb = 512
        const val MinNetworkCacheLimitMb = 64
        const val MaxNetworkCacheLimitMb = 8192
        const val BytesPerMb = 1024L * 1024L
    }
}
