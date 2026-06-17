package com.imsviewer.android.ui

import android.content.Intent
import android.graphics.Bitmap
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
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
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.ContentScale
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.imsviewer.android.domain.model.ScreenState
import com.imsviewer.android.domain.model.ViewerImage
import com.imsviewer.android.util.SizeFormatter
import com.imsviewer.android.viewmodel.ViewerViewModel
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
            ScreenState.Empty -> EmptyState(onOpenFile, onOpenFolder)
            ScreenState.Loading -> LoadingState(state.loadingMessage)
            ScreenState.Error -> ErrorState(
                message = state.errorMessage ?: "Failed to open file",
                onOpenFile = onOpenFile,
                onOpenFolder = onOpenFolder
            )
            ScreenState.Ready -> {
                val image = state.currentImage
                if (image != null) {
                    ViewerScreen(
                        image = image,
                        isUiVisible = state.isUiVisible,
                        fileIndex = state.fileIndex,
                        fileCount = state.fileSequence.size.coerceAtLeast(1),
                        thumbnails = state.thumbnails,
                        onToggleChrome = viewModel::toggleChrome,
                        onOpenFile = onOpenFile,
                        onOpenFolder = onOpenFolder,
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
            if (state.screenState == ScreenState.Ready) {
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
    }
}

@Composable
private fun EmptyState(onOpenFile: () -> Unit, onOpenFolder: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text("imsViewer", color = Color.White, fontSize = 28.sp)
        Spacer(Modifier.height(10.dp))
        Text("Open an IMS or image file to start.", color = Color(0xFFB8B8B8))
        Spacer(Modifier.height(20.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onOpenFile) {
                Text("Open file")
            }
            Button(onClick = onOpenFolder) {
                Text("Open folder")
            }
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
private fun ErrorState(message: String, onOpenFile: () -> Unit, onOpenFolder: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(message, color = Color.White)
        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onOpenFile) {
                Text("Choose file")
            }
            Button(onClick = onOpenFolder) {
                Text("Choose folder")
            }
        }
    }
}

@Composable
private fun ViewerScreen(
    image: ViewerImage,
    isUiVisible: Boolean,
    fileIndex: Int,
    fileCount: Int,
    thumbnails: Map<Int, Bitmap>,
    onToggleChrome: () -> Unit,
    onOpenFile: () -> Unit,
    onOpenFolder: () -> Unit,
    onShowInfo: () -> Unit,
    onShare: () -> Unit,
    onSubImageSelected: (Int) -> Unit,
    onPreviousSubImage: () -> Unit,
    onNextSubImage: () -> Unit,
    onPreviousFile: () -> Unit,
    onNextFile: () -> Unit
) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val isLandscape = maxWidth > maxHeight

        if (isLandscape) {
            Row(Modifier.fillMaxSize()) {
                AnimatedVisibility(visible = isUiVisible) {
                    SideChrome(
                        image = image,
                        onOpenFile = onOpenFile,
                        onOpenFolder = onOpenFolder,
                        onShowInfo = onShowInfo,
                        onShare = onShare,
                        thumbnails = thumbnails,
                        onSubImageSelected = onSubImageSelected
                    )
                }
                ImageCanvas(
                    image = image,
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
                TopChrome(image.fileName, onOpenFile, onOpenFolder)
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
                        onSubImageSelected = onSubImageSelected
                    )
                }
            }
        }
    }
}

@Composable
private fun ImageCanvas(
    image: ViewerImage,
    modifier: Modifier,
    onTap: () -> Unit,
    onPreviousSubImage: () -> Unit,
    onNextSubImage: () -> Unit,
    onPreviousFile: () -> Unit,
    onNextFile: () -> Unit
) {
    var scale by remember(image.uri, image.subImageIndex) { mutableFloatStateOf(1f) }
    var offset by remember(image.uri, image.subImageIndex) { mutableStateOf(Offset.Zero) }

    BoxWithConstraints(modifier.background(Color.Black)) {
        val density = LocalDensity.current
        val viewportWidth = with(density) { maxWidth.toPx() }
        val viewportHeight = with(density) { maxHeight.toPx() }
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
        }

        Box(
            modifier = Modifier
                .fillMaxSize()
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
                .pointerInput(image.uri, image.subImageIndex, viewportWidth, viewportHeight) {
                    awaitEachGesture {
                        awaitFirstDown(requireUnconsumed = false)
                        var singleFingerDrag = Offset.Zero
                        var hadMultiTouch = false

                        while (true) {
                            val event = awaitPointerEvent()
                            val pressedChanges = event.changes.filter { it.pressed }
                            if (pressedChanges.isEmpty()) break

                            if (pressedChanges.size >= 2) {
                                hadMultiTouch = true
                                singleFingerDrag = Offset.Zero
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
                                val delta = change.positionChange()
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
                                    }
                                    change.consume()
                                }
                            }
                        }

                        if (!hadMultiTouch && scale <= 1.02f) {
                            handleFitModeSwipe(
                                drag = singleFingerDrag,
                                viewportWidth = viewportWidth,
                                viewportHeight = viewportHeight,
                                hasSubImages = image.subImageCount > 1,
                                onPreviousSubImage = onPreviousSubImage,
                                onNextSubImage = onNextSubImage,
                                onPreviousFile = onPreviousFile,
                                onNextFile = onNextFile
                            )
                        }
                    }
                },
            contentAlignment = Alignment.Center
        ) {
            Image(
                bitmap = image.bitmap.asImageBitmap(),
                contentDescription = image.fileName,
                modifier = Modifier
                    .fillMaxSize()
                    .graphicsLayer {
                        scaleX = scale
                        scaleY = scale
                        translationX = offset.x
                        translationY = offset.y
                    },
                contentScale = ContentScale.Fit
            )
        }
    }
}

private fun handleFitModeSwipe(
    drag: Offset,
    viewportWidth: Float,
    viewportHeight: Float,
    hasSubImages: Boolean,
    onPreviousSubImage: () -> Unit,
    onNextSubImage: () -> Unit,
    onPreviousFile: () -> Unit,
    onNextFile: () -> Unit
) {
    val absX = abs(drag.x)
    val absY = abs(drag.y)
    val horizontalThreshold = viewportWidth * 0.16f
    val verticalThreshold = viewportHeight * 0.10f

    if (absX > horizontalThreshold && absX > absY * 1.35f) {
        if (drag.x < 0f) onNextFile() else onPreviousFile()
        return
    }

    if (hasSubImages && absY > verticalThreshold && absY > absX * 1.25f) {
        if (drag.y < 0f) onNextSubImage() else onPreviousSubImage()
    }
}

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
private fun TopChrome(fileName: String, onOpenFile: () -> Unit, onOpenFolder: () -> Unit) {
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
            TextButton(onClick = onOpenFile) { Text("Open") }
            TextButton(onClick = onOpenFolder) { Text("Folder") }
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
        Text(
            text = "${fileIndex + 1} / $fileCount",
            color = Color.White,
            fontSize = 13.sp
        )
        Spacer(Modifier.weight(1f))
        if (image.subImageCount > 1) {
            Text(
                text = "Sub ${image.subImageIndex + 1} / ${image.subImageCount}",
                color = Color(0xFFCCCCCC),
                fontSize = 13.sp,
                modifier = Modifier.padding(end = 12.dp)
            )
        }
        Text("${image.width} x ${image.height}", color = Color(0xFFCCCCCC), fontSize = 13.sp)
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
    onOpenFile: () -> Unit,
    onOpenFolder: () -> Unit,
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
        TextButton(onClick = onOpenFile) { Text("Open") }
        TextButton(onClick = onOpenFolder) { Text("Folder") }
        ThumbnailStrip(
            image = image,
            thumbnails = thumbnails,
            vertical = true,
            onSubImageSelected = onSubImageSelected
        )
        Spacer(Modifier.weight(1f))
        Text(
            text = if (image.subImageCount > 1) "${image.subImageIndex + 1} / ${image.subImageCount}" else "1 / 1",
            color = Color.White,
            fontSize = 13.sp
        )
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

    if (vertical) {
        LazyColumn(
            modifier = Modifier.fillMaxWidth().fillMaxHeight(0.72f),
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
