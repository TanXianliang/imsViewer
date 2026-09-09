package com.imsviewer.android.ui

import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import java.text.DateFormat
import java.util.Date
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed as lazyItemsIndexed
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.activity.compose.BackHandler
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ViewList
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.GridView
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.PhotoLibrary
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.layout.ContentScale
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.imsviewer.android.domain.model.BrowserDisplayMode
import com.imsviewer.android.domain.model.OpenRecord
import com.imsviewer.android.domain.model.OpenRecordKind
import com.imsviewer.android.domain.model.RemoteEntry
import com.imsviewer.android.domain.model.ScreenState
import com.imsviewer.android.domain.model.ViewerImage
import com.imsviewer.android.domain.wrappedAdjacentIndex
import com.imsviewer.android.util.SizeFormatter
import com.imsviewer.android.viewmodel.ViewerViewModel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

@Composable
fun MainScreen(
    viewModel: ViewerViewModel,
    onOpenFile: () -> Unit,
    onOpenFolder: () -> Unit,
    onShare: (Intent) -> Unit
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        when (state.screenState) {
            ScreenState.Empty -> HomeScreen(
                fileRecords = state.fileRecords,
                folderRecords = state.folderRecords,
                networkRecords = state.networkRecords,
                selectedKind = state.homeTab,
                networkCacheLimitMb = state.networkCacheLimitMb,
                onSelectKind = viewModel::selectHomeTab,
                onOpenFile = onOpenFile,
                onOpenFolder = onOpenFolder,
                onOpenRecord = viewModel::openRecord,
                onDeleteRecord = viewModel::deleteRecord,
                onClearRecords = viewModel::clearRecords,
                onAddNetwork = viewModel::addNetworkRecord,
                onUpdateNetwork = viewModel::updateNetworkRecord,
                onUpdateNetworkCacheLimit = viewModel::updateNetworkCacheLimit,
                onClearNetworkCache = viewModel::clearNetworkCache
            )
            ScreenState.Remote -> {
                BackHandler(onBack = viewModel::navigateRemoteBack)
                RemoteBrowserScreen(
                    title = state.remoteTitle ?: "SFTP",
                    path = state.remotePath,
                    entries = state.remoteEntries,
                    thumbnails = state.remoteThumbnails,
                    failedThumbnails = state.failedRemoteThumbnails,
                    isBusy = state.isRemoteBusy,
                    displayMode = state.remoteDisplayMode,
                    onBack = viewModel::navigateRemoteBack,
                    onHome = viewModel::showHome,
                    onOpenEntry = viewModel::openRemoteEntry,
                    onVisibleRangeChanged = viewModel::updateRemoteVisibleRange,
                    onDisplayModeChanged = viewModel::updateRemoteDisplayMode
                )
            }
            ScreenState.Folder -> {
                BackHandler(onBack = viewModel::showHome)
                FolderGridScreen(
                    title = state.folderTitle ?: "Folder",
                    uris = state.folderSequence,
                    thumbnails = state.folderThumbnails,
                    failedThumbnails = state.failedFolderThumbnails,
                    selectedIndex = state.fileIndex,
                    displayMode = state.folderDisplayMode,
                    onHome = viewModel::showHome,
                    onOpenImage = viewModel::openFolderImage,
                    onDisplayModeChanged = viewModel::updateFolderDisplayMode
                )
            }
            ScreenState.Loading -> LoadingState(state.loadingMessage)
            ScreenState.Error -> ErrorState(
                message = state.errorMessage ?: "Failed to open file",
                onHome = viewModel::showHome
            )
            ScreenState.Ready -> {
                val image = state.currentImage
                if (image != null) {
                    val hasRemoteSequence = state.remoteConnection != null && state.remoteFileSequence.isNotEmpty()
                    val activeFileIndex = if (hasRemoteSequence) {
                        state.remoteFileIndex.coerceAtLeast(0)
                    } else {
                        state.fileIndex
                    }
                    val activeFileCount = if (hasRemoteSequence) {
                        state.remoteFileSequence.size
                    } else {
                        state.fileSequence.size.coerceAtLeast(1)
                    }
                    val previousFileIndex = wrappedAdjacentIndex(activeFileIndex, activeFileCount, -1)
                    val nextFileIndex = wrappedAdjacentIndex(activeFileIndex, activeFileCount, 1)
                    val previousFilePreview = previousFileIndex?.let { index ->
                        if (hasRemoteSequence) {
                            state.remoteFileSequence.getOrNull(index)?.let { state.remoteThumbnails[it.path] }
                        } else {
                            state.fileSequence.getOrNull(index)?.let { state.folderThumbnails[it] }
                        }
                    }
                    val nextFilePreview = nextFileIndex?.let { index ->
                        if (hasRemoteSequence) {
                            state.remoteFileSequence.getOrNull(index)?.let { state.remoteThumbnails[it.path] }
                        } else {
                            state.fileSequence.getOrNull(index)?.let { state.folderThumbnails[it] }
                        }
                    }
                    BackHandler(
                        onBack = if (state.folderSequence.isNotEmpty()) {
                            viewModel::showFolder
                        } else if (state.remoteConnection != null) {
                            viewModel::showRemote
                        } else {
                            viewModel::showHome
                        }
                    )
                    ViewerScreen(
                        image = image,
                        isUiVisible = state.isUiVisible,
                        isSwitchingFile = state.isSwitchingFile,
                        isSwitchingSubImage = state.isSwitchingSubImage,
                        fileIndex = activeFileIndex,
                        fileCount = activeFileCount,
                        previousFilePreview = previousFilePreview,
                        nextFilePreview = nextFilePreview,
                        thumbnails = state.thumbnails,
                        onToggleChrome = viewModel::toggleChrome,
                        onHome = when {
                            state.folderSequence.isNotEmpty() -> viewModel::showFolder
                            state.remoteConnection != null -> viewModel::showRemote
                            else -> viewModel::showHome
                        },
                        onShowInfo = viewModel::showInfoSheet,
                        onShare = {
                            viewModel.shareIntent()?.let(onShare)
                        },
                        onSubImageSelected = viewModel::openSubImage,
                        onPreviousSubImage = viewModel::openPreviousSubImage,
                        onNextSubImage = viewModel::openNextSubImage,
                        onPreviousFile = viewModel::openPreviousFile,
                        onNextFile = viewModel::openNextFile
                    )
                }
            }
        }

        state.errorMessage?.let { message ->
            if (state.screenState == ScreenState.Ready || state.screenState == ScreenState.Empty) {
                Surface(
                    modifier = Modifier.align(Alignment.TopCenter).padding(top = 24.dp),
                    color = Color(0xDD2B1B1B)
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(message, color = Color.White, fontSize = 13.sp)
                        TextButton(onClick = viewModel::consumeError) { Text("Close") }
                    }
                }
            }
        }

        if (state.isInfoSheetVisible && state.currentImage != null) {
            ImageInfoSheet(
                image = state.currentImage!!,
                onDismiss = viewModel::hideInfoSheet
            )
        }

        state.passwordPromptRecord?.let { record ->
            NetworkPasswordSheet(
                record = record,
                onDismiss = viewModel::dismissNetworkPasswordPrompt,
                onSubmit = viewModel::submitNetworkPassword
            )
        }
    }
}

@Composable
private fun RemoteBrowserScreen(
    title: String,
    path: String,
    entries: List<RemoteEntry>,
    thumbnails: Map<String, Bitmap>,
    failedThumbnails: Set<String>,
    isBusy: Boolean,
    displayMode: BrowserDisplayMode,
    onBack: () -> Unit,
    onHome: () -> Unit,
    onOpenEntry: (RemoteEntry) -> Unit,
    onVisibleRangeChanged: (IntRange) -> Unit,
    onDisplayModeChanged: (BrowserDisplayMode) -> Unit
) {
    val listState = rememberLazyListState()
    val gridState = rememberLazyGridState()

    LaunchedEffect(path, displayMode) {
        if (displayMode == BrowserDisplayMode.List) {
            listState.scrollToItem(0)
        } else {
            gridState.scrollToItem(0)
        }
    }
    LaunchedEffect(path, entries, displayMode, listState) {
        if (displayMode != BrowserDisplayMode.List) return@LaunchedEffect
        snapshotFlow {
            val visibleItems = listState.layoutInfo.visibleItemsInfo
            if (visibleItems.isEmpty()) {
                null
            } else {
                visibleItems.first().index..visibleItems.last().index
            }
        }
            .distinctUntilChanged()
            .collect { visibleRange ->
                if (visibleRange != null) onVisibleRangeChanged(visibleRange)
            }
    }
    LaunchedEffect(path, entries, displayMode, gridState) {
        if (displayMode == BrowserDisplayMode.List) return@LaunchedEffect
        snapshotFlow {
            val visibleItems = gridState.layoutInfo.visibleItemsInfo
            if (visibleItems.isEmpty()) {
                null
            } else {
                visibleItems.first().index..visibleItems.last().index
            }
        }
            .distinctUntilChanged()
            .collect { visibleRange ->
                if (visibleRange != null) onVisibleRangeChanged(visibleRange)
            }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(Color(0xFF0E0E11))
            .windowInsetsPadding(WindowInsets.statusBars)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().height(56.dp).padding(start = 8.dp, end = 8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.Outlined.FolderOpen, contentDescription = "Back", tint = Color.White)
            }
            Column(Modifier.weight(1f)) {
                Text(title, color = Color.White, fontSize = 18.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(path, color = Color(0xFF9A9AA0), fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            BrowserDisplayModeButton(
                displayMode = displayMode,
                onDisplayModeChanged = onDisplayModeChanged
            )
            IconButton(onClick = onHome) {
                Icon(Icons.Outlined.Home, contentDescription = "Home", tint = Color.White)
            }
        }

        if (isBusy) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                Text("Loading", color = Color.White, fontSize = 13.sp)
            }
        }

        if (entries.isEmpty() && !isBusy) {
            Column(
                modifier = Modifier.fillMaxSize().padding(28.dp),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text("No supported files", color = Color.White, fontSize = 18.sp)
                Spacer(Modifier.height(8.dp))
                Text("Directories and supported image files appear here.", color = Color(0xFF9A9AA0), fontSize = 13.sp)
            }
        } else {
            if (displayMode == BrowserDisplayMode.List) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    items(entries, key = { it.path }) { entry ->
                        RemoteEntryRow(
                            entry = entry,
                            thumbnail = thumbnails[entry.path],
                            thumbnailFailed = entry.path in failedThumbnails,
                            onClick = { onOpenEntry(entry) }
                        )
                    }
                }
            } else {
                val cellSize = displayMode.minimumCellSize()
                val spacing = displayMode.gridSpacing()
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(cellSize),
                    state = gridState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(spacing),
                    horizontalArrangement = Arrangement.spacedBy(spacing),
                    verticalArrangement = Arrangement.spacedBy(spacing)
                ) {
                    itemsIndexed(entries, key = { _, entry -> entry.path }) { _, entry ->
                        RemoteEntryGridItem(
                            entry = entry,
                            thumbnail = thumbnails[entry.path],
                            thumbnailFailed = entry.path in failedThumbnails,
                            compact = displayMode == BrowserDisplayMode.SmallIcons,
                            onClick = { onOpenEntry(entry) }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun RemoteEntryRow(
    entry: RemoteEntry,
    thumbnail: Bitmap?,
    thumbnailFailed: Boolean,
    onClick: () -> Unit
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = Color(0xFF19191E),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(8.dp)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            RemoteEntryThumb(entry = entry, thumbnail = thumbnail, thumbnailFailed = thumbnailFailed)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(entry.name, color = Color.White, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (!entry.isDirectory) {
                    Text(SizeFormatter.format(entry.sizeBytes), color = Color(0xFF9A9AA0), fontSize = 12.sp)
                }
            }
        }
    }
}

@Composable
private fun RemoteEntryGridItem(
    entry: RemoteEntry,
    thumbnail: Bitmap?,
    thumbnailFailed: Boolean,
    compact: Boolean,
    onClick: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(1f)
                .background(Color(0xFF1D1D22))
                .border(1.dp, Color(0xFF333338)),
            contentAlignment = Alignment.Center
        ) {
            when {
                entry.isDirectory -> Icon(
                    imageVector = Icons.Outlined.FolderOpen,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.fillMaxSize(0.42f)
                )
                thumbnail != null -> Image(
                    bitmap = thumbnail.asImageBitmap(),
                    contentDescription = entry.name,
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop
                )
                thumbnailFailed -> Icon(
                    imageVector = Icons.Outlined.PhotoLibrary,
                    contentDescription = null,
                    tint = Color(0xFFB8B8B8),
                    modifier = Modifier.fillMaxSize(0.32f)
                )
                else -> CircularProgressIndicator(
                    modifier = Modifier.size(if (compact) 18.dp else 24.dp),
                    strokeWidth = 2.dp
                )
            }
        }
        Spacer(Modifier.height(if (compact) 3.dp else 5.dp))
        Text(
            text = entry.name,
            color = Color(0xFFD8D8DC),
            fontSize = if (compact) 10.sp else 12.sp,
            maxLines = if (compact) 1 else 2,
            overflow = TextOverflow.Ellipsis
        )
        if (!entry.isDirectory && !compact) {
            Text(
                text = SizeFormatter.format(entry.sizeBytes),
                color = Color(0xFF8F8F96),
                fontSize = 10.sp,
                maxLines = 1
            )
        }
    }
}

@Composable
private fun FolderGridScreen(
    title: String,
    uris: List<Uri>,
    thumbnails: Map<Uri, Bitmap>,
    failedThumbnails: Set<Uri>,
    selectedIndex: Int,
    displayMode: BrowserDisplayMode,
    onHome: () -> Unit,
    onOpenImage: (Int) -> Unit,
    onDisplayModeChanged: (BrowserDisplayMode) -> Unit
) {
    val gridState = rememberLazyGridState()
    val listState = rememberLazyListState()
    LaunchedEffect(selectedIndex, uris.size, displayMode) {
        if (selectedIndex in uris.indices) {
            if (displayMode == BrowserDisplayMode.List) {
                listState.scrollToItem(selectedIndex)
            } else {
                gridState.scrollToItem(selectedIndex)
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(Color(0xFF0E0E11))
            .windowInsetsPadding(WindowInsets.statusBars)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().height(56.dp).padding(start = 8.dp, end = 16.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onHome) {
                Icon(Icons.Outlined.Home, contentDescription = "Home", tint = Color.White)
            }
            Text(
                text = title,
                color = Color.White,
                fontSize = 20.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f)
            )
            Text("${uris.size}", color = Color(0xFFB8B8B8), fontSize = 13.sp)
            BrowserDisplayModeButton(
                displayMode = displayMode,
                onDisplayModeChanged = onDisplayModeChanged
            )
        }
        if (displayMode == BrowserDisplayMode.List) {
            LazyColumn(
                state = listState,
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                lazyItemsIndexed(uris, key = { _, uri -> uri.toString() }) { index, uri ->
                    FolderEntryRow(
                        index = index,
                        uri = uri,
                        bitmap = thumbnails[uri],
                        failed = uri in failedThumbnails,
                        selected = index == selectedIndex,
                        onClick = { onOpenImage(index) }
                    )
                }
            }
        } else {
            val cellSize = displayMode.minimumCellSize()
            val spacing = displayMode.gridSpacing()
            LazyVerticalGrid(
                columns = GridCells.Adaptive(cellSize),
                modifier = Modifier.fillMaxSize(),
                state = gridState,
                contentPadding = PaddingValues(spacing),
                horizontalArrangement = Arrangement.spacedBy(spacing),
                verticalArrangement = Arrangement.spacedBy(spacing)
            ) {
                itemsIndexed(uris, key = { _, uri -> uri.toString() }) { index, uri ->
                    FolderGridItem(
                        index = index,
                        uri = uri,
                        bitmap = thumbnails[uri],
                        failed = uri in failedThumbnails,
                        selected = index == selectedIndex,
                        compact = displayMode == BrowserDisplayMode.SmallIcons,
                        onClick = { onOpenImage(index) }
                    )
                }
            }
        }
    }
}

@Composable
private fun FolderGridItem(
    index: Int,
    uri: Uri,
    bitmap: Bitmap?,
    failed: Boolean,
    selected: Boolean,
    compact: Boolean,
    onClick: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(1f)
                .background(Color(0xFF1D1D22))
                .border(if (selected) 2.dp else 1.dp, if (selected) Color(0xFF7CC7FF) else Color(0xFF333338)),
            contentAlignment = Alignment.Center
        ) {
            if (bitmap != null) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = "Image ${index + 1}",
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.Crop
                )
            } else if (failed) {
                Text("Failed", color = Color(0xFFB8B8B8), fontSize = 12.sp)
            } else {
                CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
            }
        }
        Spacer(Modifier.height(5.dp))
        Text(
            text = uri.browserDisplayName(index),
            color = Color(0xFFD8D8DC),
            fontSize = if (compact) 10.sp else 12.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
    }
}

@Composable
private fun FolderEntryRow(
    index: Int,
    uri: Uri,
    bitmap: Bitmap?,
    failed: Boolean,
    selected: Boolean,
    onClick: () -> Unit
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = Color(0xFF19191E),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(8.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Box(
                modifier = Modifier
                    .size(48.dp)
                    .background(Color(0xFF232329))
                    .border(
                        if (selected) 2.dp else 1.dp,
                        if (selected) Color(0xFF7CC7FF) else Color(0xFF333338)
                    ),
                contentAlignment = Alignment.Center
            ) {
                when {
                    bitmap != null -> Image(
                        bitmap = bitmap.asImageBitmap(),
                        contentDescription = "Image ${index + 1}",
                        modifier = Modifier.fillMaxSize(),
                        contentScale = ContentScale.Crop
                    )
                    failed -> Icon(
                        imageVector = Icons.Outlined.PhotoLibrary,
                        contentDescription = null,
                        tint = Color(0xFFB8B8B8),
                        modifier = Modifier.size(22.dp)
                    )
                    else -> CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                }
            }
            Spacer(Modifier.width(12.dp))
            Text(
                text = uri.browserDisplayName(index),
                color = Color.White,
                fontSize = 15.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f)
            )
        }
    }
}

@Composable
private fun BrowserDisplayModeButton(
    displayMode: BrowserDisplayMode,
    onDisplayModeChanged: (BrowserDisplayMode) -> Unit
) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { expanded = true }) {
            Icon(
                imageVector = if (displayMode == BrowserDisplayMode.List) {
                    Icons.AutoMirrored.Outlined.ViewList
                } else {
                    Icons.Outlined.GridView
                },
                contentDescription = "Display mode: ${displayMode.displayLabel()}",
                tint = Color.White
            )
        }
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            BrowserDisplayMode.entries.forEach { mode ->
                DropdownMenuItem(
                    text = { Text(mode.displayLabel()) },
                    leadingIcon = {
                        Icon(
                            imageVector = if (mode == BrowserDisplayMode.List) {
                                Icons.AutoMirrored.Outlined.ViewList
                            } else {
                                Icons.Outlined.GridView
                            },
                            contentDescription = null
                        )
                    },
                    trailingIcon = {
                        if (mode == displayMode) {
                            Text("Current", color = Color(0xFF7CC7FF), fontSize = 11.sp)
                        }
                    },
                    onClick = {
                        expanded = false
                        if (mode != displayMode) onDisplayModeChanged(mode)
                    }
                )
            }
        }
    }
}

private fun BrowserDisplayMode.displayLabel(): String = when (this) {
    BrowserDisplayMode.LargeIcons -> "Large icons"
    BrowserDisplayMode.MediumIcons -> "Medium icons"
    BrowserDisplayMode.SmallIcons -> "Small icons"
    BrowserDisplayMode.List -> "List"
}

private fun BrowserDisplayMode.minimumCellSize() = when (this) {
    BrowserDisplayMode.LargeIcons -> 168.dp
    BrowserDisplayMode.MediumIcons -> 112.dp
    BrowserDisplayMode.SmallIcons -> 76.dp
    BrowserDisplayMode.List -> 112.dp
}

private fun BrowserDisplayMode.gridSpacing() = when (this) {
    BrowserDisplayMode.LargeIcons -> 12.dp
    BrowserDisplayMode.MediumIcons -> 10.dp
    BrowserDisplayMode.SmallIcons -> 6.dp
    BrowserDisplayMode.List -> 8.dp
}

private fun Uri.browserDisplayName(index: Int): String {
    return lastPathSegment?.substringAfterLast('/') ?: "Image ${index + 1}"
}

@Composable
private fun HomeScreen(
    fileRecords: List<OpenRecord>,
    folderRecords: List<OpenRecord>,
    networkRecords: List<OpenRecord>,
    selectedKind: OpenRecordKind,
    networkCacheLimitMb: Int,
    onSelectKind: (OpenRecordKind) -> Unit,
    onOpenFile: () -> Unit,
    onOpenFolder: () -> Unit,
    onOpenRecord: (OpenRecord) -> Unit,
    onDeleteRecord: (OpenRecord) -> Unit,
    onClearRecords: (OpenRecordKind) -> Unit,
    onAddNetwork: (String, String, String, String, String, Boolean, String) -> Unit,
    onUpdateNetwork: (OpenRecord, String, String, String, String, String, Boolean, String) -> Unit,
    onUpdateNetworkCacheLimit: (String) -> Unit,
    onClearNetworkCache: () -> Unit
) {
    var isSettingsVisible by remember { mutableStateOf(false) }
    val tabs = listOf("Open File", "Open Folder", "Open Network")
    val kinds = listOf(OpenRecordKind.File, OpenRecordKind.Folder, OpenRecordKind.Network)
    val selectedTab = kinds.indexOf(selectedKind).takeIf { it >= 0 } ?: 0
    val records = when (selectedKind) {
        OpenRecordKind.File -> fileRecords
        OpenRecordKind.Folder -> folderRecords
        OpenRecordKind.Network -> networkRecords
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(Color(0xFF0E0E11))
            .windowInsetsPadding(WindowInsets.statusBars)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().height(56.dp).padding(start = 20.dp, end = 8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("imsViewer", color = Color.White, fontSize = 22.sp, modifier = Modifier.weight(1f))
            IconButton(onClick = { isSettingsVisible = true }) {
                Icon(Icons.Outlined.Settings, contentDescription = "Settings", tint = Color.White)
            }
        }

        TabRow(
            selectedTabIndex = selectedTab,
            containerColor = Color(0xFF151519),
            contentColor = Color.White
        ) {
            tabs.forEachIndexed { index, title ->
                Tab(
                    selected = selectedTab == index,
                    onClick = { onSelectKind(kinds[index]) },
                    text = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                )
            }
        }

        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            when (selectedTab) {
                0 -> Button(
                    onClick = {
                        onSelectKind(OpenRecordKind.File)
                        onOpenFile()
                    },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Outlined.PhotoLibrary, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Open file")
                }
                1 -> Button(
                    onClick = {
                        onSelectKind(OpenRecordKind.Folder)
                        onOpenFolder()
                    },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Outlined.FolderOpen, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Open folder")
                }
                else -> Button(
                    onClick = {
                        onSelectKind(OpenRecordKind.Network)
                        isSettingsVisible = true
                    },
                    modifier = Modifier.weight(1f)
                ) {
                    Icon(Icons.Outlined.Add, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Add network")
                }
            }
        }

        RecordList(
            records = records,
            kind = kinds[selectedTab],
            onOpenRecord = onOpenRecord,
            onDeleteRecord = onDeleteRecord,
            onUpdateNetwork = onUpdateNetwork,
            networkCacheLimitMb = networkCacheLimitMb,
            onUpdateNetworkCacheLimit = onUpdateNetworkCacheLimit,
            onClearNetworkCache = onClearNetworkCache
        )
    }

    if (isSettingsVisible) {
        HomeSettingsSheet(
            selectedKind = kinds[selectedTab],
            editRecord = null,
            networkCacheLimitMb = networkCacheLimitMb,
            onDismiss = { isSettingsVisible = false },
            onAddNetwork = onAddNetwork,
            onUpdateNetwork = onUpdateNetwork,
            onClearRecords = onClearRecords,
            onUpdateNetworkCacheLimit = onUpdateNetworkCacheLimit,
            onClearNetworkCache = onClearNetworkCache
        )
    }
}

@Composable
private fun RemoteEntryThumb(entry: RemoteEntry, thumbnail: Bitmap?, thumbnailFailed: Boolean) {
    Box(
        modifier = Modifier
            .size(48.dp)
            .background(Color(0xFF232329)),
        contentAlignment = Alignment.Center
    ) {
        when {
            thumbnail != null -> Image(
                bitmap = thumbnail.asImageBitmap(),
                contentDescription = null,
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
            thumbnailFailed || entry.isDirectory -> Icon(
                imageVector = if (entry.isDirectory) Icons.Outlined.FolderOpen else Icons.Outlined.PhotoLibrary,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(22.dp)
            )
            else -> Icon(
                imageVector = Icons.Outlined.PhotoLibrary,
                contentDescription = null,
                tint = Color(0xFFB8B8B8),
                modifier = Modifier.size(22.dp)
            )
        }
    }
}

@Composable
private fun LoadingState(message: String) {
    Column(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        CircularProgressIndicator()
        Spacer(Modifier.height(12.dp))
        Text(message, color = Color.White)
    }
}

@Composable
private fun ErrorState(message: String, onHome: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(message, color = Color.White)
        Spacer(Modifier.height(16.dp))
        Button(onClick = onHome) {
            Text("Back home")
        }
    }
}

@Composable
private fun RecordList(
    records: List<OpenRecord>,
    kind: OpenRecordKind,
    onOpenRecord: (OpenRecord) -> Unit,
    onDeleteRecord: (OpenRecord) -> Unit,
    onUpdateNetwork: (OpenRecord, String, String, String, String, String, Boolean, String) -> Unit,
    networkCacheLimitMb: Int,
    onUpdateNetworkCacheLimit: (String) -> Unit,
    onClearNetworkCache: () -> Unit
) {
    var editRecord by remember { mutableStateOf<OpenRecord?>(null) }

    if (records.isEmpty()) {
        Column(
            modifier = Modifier.fillMaxSize().padding(28.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text(
                text = when (kind) {
                    OpenRecordKind.File -> "No file records"
                    OpenRecordKind.Folder -> "No folder records"
                    OpenRecordKind.Network -> "No network records"
                },
                color = Color.White,
                fontSize = 18.sp
            )
            Spacer(Modifier.height(8.dp))
            Text("Records will appear here after use.", color = Color(0xFF9A9AA0), fontSize = 13.sp)
        }
        return
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        items(records, key = { it.id }) { record ->
            RecordRow(
                record = record,
                onOpen = { onOpenRecord(record) },
                onEdit = { editRecord = record },
                onDelete = { onDeleteRecord(record) }
            )
        }
    }

    editRecord?.let { record ->
        HomeSettingsSheet(
            selectedKind = OpenRecordKind.Network,
            editRecord = record,
            networkCacheLimitMb = networkCacheLimitMb,
            onDismiss = { editRecord = null },
            onAddNetwork = { _, _, _, _, _, _, _ -> },
            onUpdateNetwork = { targetRecord, alias, host, port, username, password, savePassword, path ->
                onUpdateNetwork(targetRecord, alias, host, port, username, password, savePassword, path)
            },
            onClearRecords = {},
            onUpdateNetworkCacheLimit = onUpdateNetworkCacheLimit,
            onClearNetworkCache = onClearNetworkCache
        )
    }
}

@Composable
private fun RecordRow(record: OpenRecord, onOpen: () -> Unit, onEdit: () -> Unit, onDelete: () -> Unit) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = Color(0xFF19191E),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(8.dp)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().clickable(onClick = onOpen).padding(start = 14.dp, end = 4.dp, top = 10.dp, bottom = 10.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column(Modifier.weight(1f)) {
                Text(record.title, color = Color.White, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(3.dp))
                Text(record.subtitle, color = Color(0xFF9A9AA0), fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(3.dp))
                Text(formatRecordTime(record.updatedAt), color = Color(0xFF77777D), fontSize = 11.sp, maxLines = 1)
            }
            if (record.kind == OpenRecordKind.Network) {
                IconButton(onClick = onEdit) {
                    Icon(Icons.Outlined.MoreVert, contentDescription = "Network settings", tint = Color.White)
                }
            }
            IconButton(onClick = onDelete) {
                Icon(Icons.Outlined.Delete, contentDescription = "Delete", tint = Color.White)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HomeSettingsSheet(
    selectedKind: OpenRecordKind,
    editRecord: OpenRecord?,
    networkCacheLimitMb: Int,
    onDismiss: () -> Unit,
    onAddNetwork: (String, String, String, String, String, Boolean, String) -> Unit,
    onUpdateNetwork: (OpenRecord, String, String, String, String, String, Boolean, String) -> Unit,
    onClearRecords: (OpenRecordKind) -> Unit,
    onUpdateNetworkCacheLimit: (String) -> Unit,
    onClearNetworkCache: () -> Unit
) {
    var alias by remember(editRecord) { mutableStateOf(editRecord?.alias.orEmpty()) }
    var host by remember(editRecord) { mutableStateOf(editRecord?.host.orEmpty()) }
    var port by remember(editRecord) { mutableStateOf(editRecord?.port?.toString() ?: "22") }
    var username by remember(editRecord) { mutableStateOf(editRecord?.username.orEmpty()) }
    var password by remember(editRecord) { mutableStateOf("") }
    var savePassword by remember(editRecord) { mutableStateOf(editRecord?.hasSavedPassword == true) }
    var isPasswordVisible by remember(editRecord) { mutableStateOf(false) }
    var path by remember(editRecord) { mutableStateOf(editRecord?.path ?: "/") }
    var cacheLimitMb by remember(networkCacheLimitMb) { mutableStateOf(networkCacheLimitMb.toString()) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        dragHandle = { BottomSheetDefaults.DragHandle() },
        containerColor = Color(0xFF17171B)
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Settings", color = Color.White, fontSize = 20.sp, modifier = Modifier.weight(1f))
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Outlined.Close, contentDescription = "Close", tint = Color.White)
                }
            }
            if (selectedKind == OpenRecordKind.Network) {
                TextField(
                    value = alias,
                    onValueChange = { alias = it },
                    label = { Text("Name") },
                    placeholder = { Text("Optional, defaults to host:port") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(10.dp))
                TextField(
                    value = host,
                    onValueChange = { host = it },
                    label = { Text("Host") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(10.dp))
                TextField(
                    value = port,
                    onValueChange = { port = it.filter(Char::isDigit).take(5) },
                    label = { Text("Port") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(10.dp))
                TextField(
                    value = username,
                    onValueChange = { username = it },
                    label = { Text("Username") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(10.dp))
                TextField(
                    value = password,
                    onValueChange = { password = it },
                    label = { Text("Password") },
                    placeholder = {
                        Text(
                            when {
                                editRecord?.hasSavedPassword == true && savePassword -> "Leave blank to keep saved password"
                                savePassword -> "Saved securely on this device"
                                else -> "Required when opening if not saved"
                            }
                        )
                    },
                    singleLine = true,
                    visualTransformation = if (isPasswordVisible) {
                        VisualTransformation.None
                    } else {
                        PasswordVisualTransformation()
                    },
                    trailingIcon = {
                        PasswordVisibilityButton(
                            isVisible = isPasswordVisible,
                            onToggle = { isPasswordVisible = !isPasswordVisible }
                        )
                    },
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(10.dp))
                Row(
                    modifier = Modifier.fillMaxWidth().clickable { savePassword = !savePassword },
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Checkbox(checked = savePassword, onCheckedChange = { savePassword = it })
                    Text("Save password", color = Color.White, fontSize = 14.sp)
                }
                Spacer(Modifier.height(10.dp))
                TextField(
                    value = path,
                    onValueChange = { path = it },
                    label = { Text("Remote path") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(12.dp))
                TextField(
                    value = cacheLimitMb,
                    onValueChange = { cacheLimitMb = it.filter(Char::isDigit).take(5) },
                    label = { Text("Network cache limit (MB)") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(Modifier.height(10.dp))
                TextButton(
                    onClick = onClearNetworkCache,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text("Clear network cache")
                }
                Spacer(Modifier.height(12.dp))
                Button(
                    onClick = {
                        onUpdateNetworkCacheLimit(cacheLimitMb)
                        if (editRecord == null) {
                            onAddNetwork(alias, host, port, username, password, savePassword, path)
                        } else {
                            onUpdateNetwork(editRecord, alias, host, port, username, password, savePassword, path)
                        }
                        onDismiss()
                    },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(if (editRecord == null) "Add network record" else "Save network record")
                }
                Spacer(Modifier.height(14.dp))
            }
            if (editRecord == null) {
                TextButton(
                    onClick = {
                        onClearRecords(selectedKind)
                        onDismiss()
                    },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        text = when (selectedKind) {
                            OpenRecordKind.File -> "Clear file records"
                            OpenRecordKind.Folder -> "Clear folder records"
                            OpenRecordKind.Network -> "Clear network records"
                        }
                    )
                }
            }
            Spacer(Modifier.height(20.dp))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NetworkPasswordSheet(
    record: OpenRecord,
    onDismiss: () -> Unit,
    onSubmit: (OpenRecord, String, Boolean) -> Unit
) {
    var password by remember(record.id) { mutableStateOf("") }
    var savePassword by remember(record.id) { mutableStateOf(false) }
    var isPasswordVisible by remember(record.id) { mutableStateOf(false) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        dragHandle = { BottomSheetDefaults.DragHandle() },
        containerColor = Color(0xFF17171B)
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Password", color = Color.White, fontSize = 20.sp, modifier = Modifier.weight(1f))
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Outlined.Close, contentDescription = "Close", tint = Color.White)
                }
            }
            Text(record.title, color = Color(0xFFCCCCCC), fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Spacer(Modifier.height(12.dp))
            TextField(
                value = password,
                onValueChange = { password = it },
                label = { Text("SFTP password") },
                singleLine = true,
                visualTransformation = if (isPasswordVisible) {
                    VisualTransformation.None
                } else {
                    PasswordVisualTransformation()
                },
                trailingIcon = {
                    PasswordVisibilityButton(
                        isVisible = isPasswordVisible,
                        onToggle = { isPasswordVisible = !isPasswordVisible }
                    )
                },
                modifier = Modifier.fillMaxWidth()
            )
            Spacer(Modifier.height(12.dp))
            Row(
                modifier = Modifier.fillMaxWidth().clickable { savePassword = !savePassword },
                verticalAlignment = Alignment.CenterVertically
            ) {
                Checkbox(checked = savePassword, onCheckedChange = { savePassword = it })
                Text("Save password", color = Color.White, fontSize = 14.sp)
            }
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = { onSubmit(record, password, savePassword) },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Connect")
            }
            Spacer(Modifier.height(20.dp))
        }
    }
}

@Composable
private fun PasswordVisibilityButton(isVisible: Boolean, onToggle: () -> Unit) {
    IconButton(onClick = onToggle) {
        Icon(
            imageVector = if (isVisible) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
            contentDescription = if (isVisible) "Hide password" else "Show password"
        )
    }
}

private fun formatRecordTime(updatedAt: Long): String {
    if (updatedAt <= 0L) return ""
    return DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(updatedAt))
}

@Composable
private fun ViewerScreen(
    image: ViewerImage,
    isUiVisible: Boolean,
    isSwitchingFile: Boolean,
    isSwitchingSubImage: Boolean,
    fileIndex: Int,
    fileCount: Int,
    previousFilePreview: Bitmap?,
    nextFilePreview: Bitmap?,
    thumbnails: Map<Int, Bitmap>,
    onToggleChrome: () -> Unit,
    onHome: () -> Unit,
    onShowInfo: () -> Unit,
    onShare: () -> Unit,
    onSubImageSelected: (Int) -> Unit,
    onPreviousSubImage: () -> Unit,
    onNextSubImage: () -> Unit,
    onPreviousFile: () -> Unit,
    onNextFile: () -> Unit
) {
    var isFileSwipeSettling by remember(image.uri) { mutableStateOf(false) }
    val guardedSubImageSelected: (Int) -> Unit = { index ->
        if (!isFileSwipeSettling && !isSwitchingFile) onSubImageSelected(index)
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val isLandscape = maxWidth > maxHeight

        if (isLandscape) {
            Row(Modifier.fillMaxSize()) {
                AnimatedVisibility(visible = isUiVisible) {
                    SideChrome(
                        image = image,
                        onHome = onHome,
                        fileIndex = fileIndex,
                        fileCount = fileCount,
                        onShowInfo = onShowInfo,
                        onShare = onShare,
                        thumbnails = thumbnails,
                        onSubImageSelected = guardedSubImageSelected
                    )
                }
                ImageCanvas(
                    image = image,
                    thumbnails = thumbnails,
                    isSwitchingFile = isSwitchingFile,
                    isSwitchingSubImage = isSwitchingSubImage,
                    fileCount = fileCount,
                    previousFilePreview = previousFilePreview,
                    nextFilePreview = nextFilePreview,
                    onFileSwipeSettlingChange = { isFileSwipeSettling = it },
                    modifier = Modifier.weight(1f).fillMaxHeight(),
                    onTap = onToggleChrome,
                    onPreviousSubImage = onPreviousSubImage,
                    onNextSubImage = onNextSubImage,
                    onPreviousFile = onPreviousFile,
                    onNextFile = onNextFile
                )
            }
        } else {
            ImageCanvas(
                image = image,
                thumbnails = thumbnails,
                isSwitchingFile = isSwitchingFile,
                isSwitchingSubImage = isSwitchingSubImage,
                fileCount = fileCount,
                previousFilePreview = previousFilePreview,
                nextFilePreview = nextFilePreview,
                onFileSwipeSettlingChange = { isFileSwipeSettling = it },
                modifier = Modifier.fillMaxSize(),
                onTap = onToggleChrome,
                onPreviousSubImage = onPreviousSubImage,
                onNextSubImage = onNextSubImage,
                onPreviousFile = onPreviousFile,
                onNextFile = onNextFile
            )
            AnimatedVisibility(
                visible = isUiVisible,
                modifier = Modifier.align(Alignment.TopCenter)
            ) {
                TopChrome(image.fileName, onHome)
            }
            AnimatedVisibility(
                visible = isUiVisible,
                modifier = Modifier.align(Alignment.BottomCenter)
            ) {
                Column(Modifier.fillMaxWidth()) {
                    BottomChrome(
                        image = image,
                        fileIndex = fileIndex,
                        fileCount = fileCount,
                        onShowInfo = onShowInfo,
                        onShare = onShare
                    )
                    ThumbnailStrip(
                        image = image,
                        thumbnails = thumbnails,
                        vertical = false,
                        onSubImageSelected = guardedSubImageSelected
                    )
                }
            }
        }

        AnimatedVisibility(
            visible = isSwitchingFile,
            modifier = Modifier.align(Alignment.Center)
        ) {
            Surface(
                color = Color(0xCC111114),
                shape = androidx.compose.foundation.shape.RoundedCornerShape(6.dp)
            ) {
                Row(
                    modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    Text("Loading", color = Color.White, fontSize = 13.sp)
                }
            }
        }
    }
}

@Composable
private fun ImageCanvas(
    image: ViewerImage,
    thumbnails: Map<Int, Bitmap>,
    isSwitchingFile: Boolean,
    isSwitchingSubImage: Boolean,
    fileCount: Int,
    previousFilePreview: Bitmap?,
    nextFilePreview: Bitmap?,
    onFileSwipeSettlingChange: (Boolean) -> Unit,
    modifier: Modifier,
    onTap: () -> Unit,
    onPreviousSubImage: () -> Unit,
    onNextSubImage: () -> Unit,
    onPreviousFile: () -> Unit,
    onNextFile: () -> Unit
) {
    var scale by remember(image.uri, image.subImageIndex) { mutableFloatStateOf(1f) }
    var offset by remember(image.uri, image.subImageIndex) { mutableStateOf(Offset.Zero) }
    var horizontalSwipeOffset by remember(image.uri, image.subImageIndex) { mutableFloatStateOf(0f) }
    var verticalSwipeOffset by remember(image.uri, image.subImageIndex) { mutableFloatStateOf(0f) }
    var awaitingFileChange by remember(image.uri, image.subImageIndex) { mutableStateOf(false) }
    var awaitingSubImageChange by remember(image.uri, image.subImageIndex) { mutableStateOf(false) }

    BoxWithConstraints(modifier.background(Color.Black)) {
        val density = LocalDensity.current
        val viewportWidth = with(density) { maxWidth.toPx() }
        val viewportHeight = with(density) { maxHeight.toPx() }
        val maximumHorizontalThreshold = with(density) { 96.dp.toPx() }
        val maximumVerticalThreshold = with(density) { 52.dp.toPx() }
        val flingVelocityThreshold = with(density) { 900.dp.toPx() }
        val fittedImageSize = remember(image.width, image.height, viewportWidth, viewportHeight) {
            fittedSize(
                imageWidth = image.width.toFloat(),
                imageHeight = image.height.toFloat(),
                viewportWidth = viewportWidth,
                viewportHeight = viewportHeight
            )
        }

        LaunchedEffect(image.uri, image.subImageIndex, viewportWidth, viewportHeight) {
            scale = 1f
            offset = Offset.Zero
            horizontalSwipeOffset = 0f
            verticalSwipeOffset = 0f
            awaitingFileChange = false
            awaitingSubImageChange = false
        }

        LaunchedEffect(isSwitchingFile) {
            if (!isSwitchingFile && awaitingFileChange) {
                animate(
                    initialValue = horizontalSwipeOffset,
                    targetValue = 0f,
                    animationSpec = spring(
                        dampingRatio = Spring.DampingRatioNoBouncy,
                        stiffness = Spring.StiffnessMedium
                    )
                ) { value, _ -> horizontalSwipeOffset = value }
                awaitingFileChange = false
                onFileSwipeSettlingChange(false)
            }
        }

        LaunchedEffect(isSwitchingSubImage) {
            if (!isSwitchingSubImage && awaitingSubImageChange) {
                animate(
                    initialValue = verticalSwipeOffset,
                    targetValue = 0f,
                    animationSpec = spring(
                        dampingRatio = Spring.DampingRatioNoBouncy,
                        stiffness = Spring.StiffnessMedium
                    )
                ) { value, _ -> verticalSwipeOffset = value }
                awaitingSubImageChange = false
            }
        }

        Box(
            modifier = Modifier
                .fillMaxSize()
                .clipToBounds()
                .pointerInput(image.uri, image.subImageIndex, viewportWidth, viewportHeight) {
                    detectTapGestures(
                        onTap = { onTap() },
                        onDoubleTap = { tapOffset ->
                            val targetScale = if (scale > 1.05f) 1f else 2.5f
                            val nextOffset = zoomAroundPoint(
                                currentOffset = offset,
                                oldScale = scale,
                                newScale = targetScale,
                                focalPoint = tapOffset,
                                viewportWidth = viewportWidth,
                                viewportHeight = viewportHeight
                            )
                            scale = targetScale
                            offset = clampOffset(nextOffset, fittedImageSize, targetScale, viewportWidth, viewportHeight)
                        }
                    )
                }
                .pointerInput(
                    image.uri,
                    image.subImageIndex,
                    viewportWidth,
                    viewportHeight,
                    isSwitchingFile,
                    isSwitchingSubImage
                ) {
                    if (isSwitchingFile || isSwitchingSubImage) return@pointerInput
                    while (true) {
                        val gesture = awaitPointerEventScope {
                            val down = awaitFirstDown(requireUnconsumed = false)
                            var singleFingerDrag = Offset.Zero
                            var hadMultiTouch = false
                            var swipeAxis = SwipeAxis.Undecided
                            val velocityTracker = VelocityTracker().apply {
                                addPosition(down.uptimeMillis, down.position)
                            }

                            while (true) {
                                val event = awaitPointerEvent()
                                event.changes.firstOrNull { it.id == down.id }?.let { change ->
                                    velocityTracker.addPosition(change.uptimeMillis, change.position)
                                }
                                val pressedChanges = event.changes.filter { it.pressed }
                                if (pressedChanges.isEmpty()) break

                                if (pressedChanges.size >= 2) {
                                    hadMultiTouch = true
                                    singleFingerDrag = Offset.Zero
                                    swipeAxis = SwipeAxis.Undecided
                                    horizontalSwipeOffset = 0f
                                    verticalSwipeOffset = 0f
                                    val zoom = event.calculateZoomFactor()
                                    val pan = event.calculateAveragePan()
                                    val centroid = event.calculateCurrentCentroid()
                                    val nextScale = (scale * zoom).coerceIn(1f, 12f)
                                    val nextOffset = zoomAroundPoint(
                                        currentOffset = offset,
                                        oldScale = scale,
                                        newScale = nextScale,
                                        focalPoint = centroid,
                                        viewportWidth = viewportWidth,
                                        viewportHeight = viewportHeight
                                    ) + pan
                                    scale = nextScale
                                    offset = clampOffset(
                                        nextOffset,
                                        fittedImageSize,
                                        nextScale,
                                        viewportWidth,
                                        viewportHeight
                                    )
                                    event.changes.forEach { it.consume() }
                                } else {
                                    val change = pressedChanges.first()
                                    val delta = change.position - change.previousPosition
                                    if (delta != Offset.Zero) {
                                        if (scale > 1.02f) {
                                            offset = clampOffset(
                                                offset + delta,
                                                fittedImageSize,
                                                scale,
                                                viewportWidth,
                                                viewportHeight
                                            )
                                        } else {
                                            singleFingerDrag += delta
                                            if (swipeAxis == SwipeAxis.Undecided) {
                                                swipeAxis = resolveSwipeAxis(
                                                    dragX = singleFingerDrag.x,
                                                    dragY = singleFingerDrag.y,
                                                    touchSlop = viewConfiguration.touchSlop,
                                                    hasSubImages = image.subImageCount > 1
                                                )
                                            }
                                            if (swipeAxis == SwipeAxis.Vertical) {
                                                verticalSwipeOffset = singleFingerDrag.y.coerceIn(
                                                    -viewportHeight,
                                                    viewportHeight
                                                )
                                            } else if (swipeAxis == SwipeAxis.Horizontal) {
                                                horizontalSwipeOffset = singleFingerDrag.x.coerceIn(
                                                    -viewportWidth,
                                                    viewportWidth
                                                )
                                            }
                                        }
                                        change.consume()
                                    }
                                }
                            }

                            val velocity = velocityTracker.calculateVelocity()
                            CompletedFitGesture(
                                drag = singleFingerDrag,
                                velocity = Offset(velocity.x, velocity.y),
                                axis = swipeAxis,
                                hadMultiTouch = hadMultiTouch
                            )
                        }

                        if (!gesture.hadMultiTouch && scale <= 1.02f) {
                            val minimumFlingDistance = viewConfiguration.touchSlop * 1.5f
                            val action = resolveFitModeSwipe(
                                axis = gesture.axis,
                                dragX = gesture.drag.x,
                                dragY = gesture.drag.y,
                                velocityX = gesture.velocity.x,
                                velocityY = gesture.velocity.y,
                                horizontalDistanceThreshold = max(
                                    viewConfiguration.touchSlop * 2f,
                                    min(viewportWidth * 0.16f, maximumHorizontalThreshold)
                                ),
                                verticalDistanceThreshold = max(
                                    viewConfiguration.touchSlop * 2f,
                                    min(viewportHeight * 0.08f, maximumVerticalThreshold)
                                ),
                                minimumFlingDistance = minimumFlingDistance,
                                flingVelocityThreshold = flingVelocityThreshold,
                                hasSubImages = image.subImageCount > 1,
                            )
                            when (action) {
                                FitModeSwipeAction.PreviousSubImage,
                                FitModeSwipeAction.NextSubImage -> {
                                    awaitingSubImageChange = true
                                    animate(
                                        initialValue = verticalSwipeOffset,
                                        targetValue = if (action == FitModeSwipeAction.NextSubImage) {
                                            -viewportHeight
                                        } else {
                                            viewportHeight
                                        },
                                        animationSpec = tween(durationMillis = 180)
                                    ) { value, _ -> verticalSwipeOffset = value }
                                    if (action == FitModeSwipeAction.NextSubImage) {
                                        onNextSubImage()
                                    } else {
                                        onPreviousSubImage()
                                    }
                                }
                                FitModeSwipeAction.PreviousFile,
                                FitModeSwipeAction.NextFile -> {
                                    if (fileCount > 1) {
                                        awaitingFileChange = true
                                        onFileSwipeSettlingChange(true)
                                        var navigationRequested = false
                                        try {
                                            animate(
                                                initialValue = horizontalSwipeOffset,
                                                targetValue = if (action == FitModeSwipeAction.NextFile) {
                                                    -viewportWidth
                                                } else {
                                                    viewportWidth
                                                },
                                                animationSpec = tween(durationMillis = 180)
                                            ) { value, _ -> horizontalSwipeOffset = value }
                                            navigationRequested = true
                                        if (action == FitModeSwipeAction.NextFile) {
                                            onNextFile()
                                        } else {
                                            onPreviousFile()
                                        }
                                    } finally {
                                            if (!navigationRequested) {
                                                horizontalSwipeOffset = 0f
                                                awaitingFileChange = false
                                                onFileSwipeSettlingChange(false)
                                            }
                                        }
                                    } else {
                                        onFileSwipeSettlingChange(true)
                                        try {
                                            animate(
                                                initialValue = horizontalSwipeOffset,
                                                targetValue = 0f,
                                                animationSpec = spring(
                                                    dampingRatio = Spring.DampingRatioNoBouncy,
                                                    stiffness = Spring.StiffnessMedium
                                                )
                                            ) { value, _ -> horizontalSwipeOffset = value }
                                        } finally {
                                            horizontalSwipeOffset = 0f
                                            onFileSwipeSettlingChange(false)
                                        }
                                    }
                                }
                                FitModeSwipeAction.None -> {
                                    if (abs(horizontalSwipeOffset) > 0.5f) {
                                        onFileSwipeSettlingChange(true)
                                        try {
                                            animate(
                                                initialValue = horizontalSwipeOffset,
                                                targetValue = 0f,
                                                animationSpec = spring(
                                                    dampingRatio = Spring.DampingRatioNoBouncy,
                                                    stiffness = Spring.StiffnessMedium
                                                )
                                            ) { value, _ -> horizontalSwipeOffset = value }
                                        } finally {
                                            horizontalSwipeOffset = 0f
                                            onFileSwipeSettlingChange(false)
                                        }
                                    } else {
                                        horizontalSwipeOffset = 0f
                                    }
                                    animate(
                                        initialValue = verticalSwipeOffset,
                                        targetValue = 0f,
                                        animationSpec = spring(
                                            dampingRatio = Spring.DampingRatioNoBouncy,
                                            stiffness = Spring.StiffnessMedium
                                        )
                                    ) { value, _ -> verticalSwipeOffset = value }
                                }
                            }
                        } else {
                            horizontalSwipeOffset = 0f
                            verticalSwipeOffset = 0f
                        }
                    }
                },
            contentAlignment = Alignment.Center
        ) {
            val swipeOffsetX = horizontalSwipeOffset
            val swipeOffsetY = verticalSwipeOffset
            val targetFilePreview = when {
                swipeOffsetX < -0.5f -> nextFilePreview
                swipeOffsetX > 0.5f -> previousFilePreview
                else -> null
            }
            if (abs(swipeOffsetX) > 0.5f && fileCount > 1) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .graphicsLayer {
                            translationX = swipeOffsetX + if (swipeOffsetX < 0f) {
                                viewportWidth
                            } else {
                                -viewportWidth
                            }
                        },
                    contentAlignment = Alignment.Center
                ) {
                    if (targetFilePreview != null) {
                        Image(
                            bitmap = targetFilePreview.asImageBitmap(),
                            contentDescription = null,
                            modifier = Modifier.fillMaxSize(),
                            contentScale = ContentScale.Fit
                        )
                    } else {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            color = Color.White,
                            strokeWidth = 2.dp
                        )
                    }
                }
            }
            val targetSubImageIndex = when {
                swipeOffsetY < -0.5f -> (image.subImageIndex + 1) % image.subImageCount
                swipeOffsetY > 0.5f -> (image.subImageIndex - 1 + image.subImageCount) % image.subImageCount
                else -> null
            }
            targetSubImageIndex?.let { targetIndex ->
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .graphicsLayer {
                            translationY = swipeOffsetY + if (swipeOffsetY < 0f) {
                                viewportHeight
                            } else {
                                -viewportHeight
                            }
                        },
                    contentAlignment = Alignment.Center
                ) {
                    val preview = thumbnails[targetIndex]
                    if (preview != null) {
                        Image(
                            bitmap = preview.asImageBitmap(),
                            contentDescription = null,
                            modifier = Modifier.fillMaxSize(),
                            contentScale = ContentScale.Fit
                        )
                    } else {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            color = Color.White,
                            strokeWidth = 2.dp
                        )
                    }
                }
            }
            Image(
                bitmap = image.bitmap.asImageBitmap(),
                contentDescription = image.fileName,
                modifier = Modifier
                    .fillMaxSize()
                    .graphicsLayer {
                        scaleX = scale
                        scaleY = scale
                        translationX = offset.x + swipeOffsetX
                        translationY = offset.y + swipeOffsetY
                    },
                contentScale = ContentScale.Fit
            )
        }
    }
}

private data class CompletedFitGesture(
    val drag: Offset,
    val velocity: Offset,
    val axis: SwipeAxis,
    val hadMultiTouch: Boolean
)

private fun PointerEvent.calculateCurrentCentroid(): Offset {
    val pressed = changes.filter { it.pressed }
    if (pressed.isEmpty()) return Offset.Zero
    val sum = pressed.fold(Offset.Zero) { acc, change -> acc + change.position }
    return sum / pressed.size.toFloat()
}

private fun PointerEvent.calculatePreviousCentroid(): Offset {
    val pressed = changes.filter { it.pressed }
    if (pressed.isEmpty()) return Offset.Zero
    val sum = pressed.fold(Offset.Zero) { acc, change -> acc + change.previousPosition }
    return sum / pressed.size.toFloat()
}

private fun PointerEvent.calculateAveragePan(): Offset {
    val pressed = changes.filter { it.pressed }
    if (pressed.isEmpty()) return Offset.Zero
    val sum = pressed.fold(Offset.Zero) { acc, change -> acc + (change.position - change.previousPosition) }
    return sum / pressed.size.toFloat()
}

private fun PointerEvent.calculateZoomFactor(): Float {
    val pressed = changes.filter { it.pressed }
    if (pressed.size < 2) return 1f

    val currentCentroid = calculateCurrentCentroid()
    val previousCentroid = calculatePreviousCentroid()
    val currentDistance = pressed
        .map { (it.position - currentCentroid).getDistance() }
        .average()
        .toFloat()
    val previousDistance = pressed
        .map { (it.previousPosition - previousCentroid).getDistance() }
        .average()
        .toFloat()

    return if (previousDistance <= 0f || currentDistance <= 0f) 1f else currentDistance / previousDistance
}

private fun fittedSize(
    imageWidth: Float,
    imageHeight: Float,
    viewportWidth: Float,
    viewportHeight: Float
): Offset {
    if (imageWidth <= 0f || imageHeight <= 0f || viewportWidth <= 0f || viewportHeight <= 0f) {
        return Offset.Zero
    }
    val fitScale = min(viewportWidth / imageWidth, viewportHeight / imageHeight)
    return Offset(imageWidth * fitScale, imageHeight * fitScale)
}

private fun zoomAroundPoint(
    currentOffset: Offset,
    oldScale: Float,
    newScale: Float,
    focalPoint: Offset,
    viewportWidth: Float,
    viewportHeight: Float
): Offset {
    if (oldScale <= 0f || newScale <= 1.01f) return Offset.Zero
    val scaleChange = newScale / oldScale
    val viewportCenter = Offset(viewportWidth / 2f, viewportHeight / 2f)
    val focalFromCenter = focalPoint - viewportCenter
    return currentOffset * scaleChange + focalFromCenter * (1f - scaleChange)
}

private fun clampOffset(
    offset: Offset,
    fittedImageSize: Offset,
    scale: Float,
    viewportWidth: Float,
    viewportHeight: Float
): Offset {
    if (scale <= 1.01f) return Offset.Zero
    val maxX = max(0f, (fittedImageSize.x * scale - viewportWidth) / 2f)
    val maxY = max(0f, (fittedImageSize.y * scale - viewportHeight) / 2f)
    return Offset(
        x = offset.x.coerceIn(-maxX, maxX),
        y = offset.y.coerceIn(-maxY, maxY)
    )
}

@Composable
private fun TopChrome(fileName: String, onHome: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xCC111114))
            .windowInsetsPadding(WindowInsets.statusBars)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
                .padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onHome) {
                Icon(Icons.Outlined.Home, contentDescription = "Home", tint = Color.White)
            }
            Text(
                text = fileName,
                color = Color.White,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(horizontal = 8.dp)
            )
        }
    }
}

@Composable
private fun BottomChrome(
    image: ViewerImage,
    fileIndex: Int,
    fileCount: Int,
    onShowInfo: () -> Unit,
    onShare: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xCC111114))
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        val status = buildString {
            append("${fileIndex + 1} / $fileCount")
            if (image.subImageCount > 1) {
                append("  ·  Sub ${image.subImageIndex + 1} / ${image.subImageCount}")
            }
            append("  ·  ${image.width} x ${image.height}")
        }
        Text(
            text = status,
            color = Color.White,
            fontSize = 13.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f)
        )
        IconButton(onClick = onShowInfo) {
            Icon(Icons.Outlined.Info, contentDescription = "Info", tint = Color.White)
        }
        IconButton(onClick = onShare) {
            Icon(Icons.Outlined.Share, contentDescription = "Share", tint = Color.White)
        }
    }
}

@Composable
private fun SideChrome(
    image: ViewerImage,
    onHome: () -> Unit,
    fileIndex: Int,
    fileCount: Int,
    onShowInfo: () -> Unit,
    onShare: () -> Unit,
    thumbnails: Map<Int, Bitmap>,
    onSubImageSelected: (Int) -> Unit
) {
    Column(
        modifier = Modifier
            .width(132.dp)
            .fillMaxHeight()
            .background(Color(0xCC111114))
            .padding(8.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Row {
            IconButton(onClick = onHome) {
                Icon(Icons.Outlined.Home, contentDescription = "Home", tint = Color.White)
            }
        }
        ThumbnailStrip(
            image = image,
            thumbnails = thumbnails,
            vertical = true,
            onSubImageSelected = onSubImageSelected
        )
        Spacer(Modifier.weight(1f))
        Text(
            text = "${fileIndex + 1} / $fileCount",
            color = Color.White,
            fontSize = 13.sp
        )
        if (image.subImageCount > 1) {
            Text(
                text = "Sub ${image.subImageIndex + 1} / ${image.subImageCount}",
                color = Color(0xFFCCCCCC),
                fontSize = 12.sp
            )
        }
        Row {
            IconButton(onClick = onShowInfo) {
                Icon(Icons.Outlined.Info, contentDescription = "Info", tint = Color.White)
            }
            IconButton(onClick = onShare) {
                Icon(Icons.Outlined.Share, contentDescription = "Share", tint = Color.White)
            }
        }
    }
}

@Composable
private fun ThumbnailStrip(
    image: ViewerImage,
    thumbnails: Map<Int, Bitmap>,
    vertical: Boolean,
    onSubImageSelected: (Int) -> Unit
) {
    val entries = image.imsFile?.entries.orEmpty()
    if (entries.size <= 1) return
    val listState = rememberLazyListState()

    LaunchedEffect(image.subImageIndex, entries.size) {
        if (image.subImageIndex in entries.indices) {
            listState.animateScrollToItem(image.subImageIndex)
        }
    }

    if (vertical) {
        LazyColumn(
            modifier = Modifier.fillMaxWidth().fillMaxHeight(0.72f),
            state = listState,
            contentPadding = PaddingValues(vertical = 4.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            items(entries, key = { it.index }) { entry ->
                ThumbnailItem(
                    label = "${entry.index + 1}",
                    bitmap = thumbnails[entry.index],
                    selected = entry.index == image.subImageIndex,
                    onClick = { onSubImageSelected(entry.index) }
                )
            }
        }
    } else {
        LazyRow(
            modifier = Modifier.fillMaxWidth().height(74.dp).background(Color(0xCC111114)),
            state = listState,
            contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            items(entries, key = { it.index }) { entry ->
                ThumbnailItem(
                    label = "${entry.index + 1}",
                    bitmap = thumbnails[entry.index],
                    selected = entry.index == image.subImageIndex,
                    onClick = { onSubImageSelected(entry.index) }
                )
            }
        }
    }
}

@Composable
private fun ThumbnailItem(label: String, bitmap: Bitmap?, selected: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(56.dp)
            .background(Color(0xFF222226))
            .border(if (selected) 2.dp else 1.dp, if (selected) Color(0xFF7CC7FF) else Color(0xFF444448))
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        if (bitmap != null) {
            Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = "Sub image $label",
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop
            )
        } else {
            Text(label, color = Color.White, fontSize = 13.sp)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ImageInfoSheet(image: ViewerImage, onDismiss: () -> Unit) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        dragHandle = { BottomSheetDefaults.DragHandle() },
        containerColor = Color(0xFF17171B)
    ) {
        Column(Modifier.fillMaxWidth().padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Info", color = Color.White, fontSize = 20.sp, modifier = Modifier.weight(1f))
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Outlined.Close, contentDescription = "Close", tint = Color.White)
                }
            }
            InfoRow("Name", image.fileName)
            InfoRow("Format", image.format.label)
            InfoRow("Dimensions", "${image.width} x ${image.height}")
            InfoRow("Size", SizeFormatter.format(image.sizeBytes))
            InfoRow("URI", image.uri.toString(), monospace = true)
            image.imsFile?.let { ims ->
                Spacer(Modifier.height(12.dp))
                InfoRow("IMS version", ims.version.toString())
                InfoRow("Sub images", ims.entries.size.toString())
                image.currentSubImage?.let { sub ->
                    InfoRow("Current", "${sub.index + 1}: ${sub.name}")
                    InfoRow("Sub format", sub.format.label)
                    InfoRow("Sub bytes", SizeFormatter.format(sub.length))
                    InfoRow("Sub offset", sub.offset.toString())
                }
            }
            Spacer(Modifier.height(20.dp))
        }
    }
}

@Composable
private fun InfoRow(label: String, value: String, monospace: Boolean = false) {
    Column(Modifier.fillMaxWidth().padding(vertical = 5.dp)) {
        Text(label, color = Color(0xFF999999), fontSize = 12.sp)
        Text(
            value,
            color = Color.White,
            fontSize = 14.sp,
            fontFamily = if (monospace) FontFamily.Monospace else FontFamily.Default
        )
    }
}
