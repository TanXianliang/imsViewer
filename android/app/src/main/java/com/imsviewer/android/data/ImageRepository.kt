package com.imsviewer.android.data

import android.content.ContentResolver
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import com.imsviewer.android.domain.ImageFormatDetector
import com.imsviewer.android.domain.ImsParser
import com.imsviewer.android.domain.model.ImageFormat
import com.imsviewer.android.domain.model.ImsFile
import com.imsviewer.android.domain.model.SubImage
import com.imsviewer.android.domain.model.ViewerImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.Locale

class ImageRepository(
    private val context: Context,
    private val detector: ImageFormatDetector = ImageFormatDetector(),
    private val imsParser: ImsParser = ImsParser()
) {
    private val resolver: ContentResolver = context.contentResolver

    suspend fun open(uri: Uri, preferredSubImageIndex: Int = 0): ViewerImage = withContext(Dispatchers.IO) {
        val fileName = queryFileName(uri)
        val sizeBytes = querySize(uri)
        val format = detector.detect(resolver, uri, fileName)

        if (format == ImageFormat.Ims) {
            openIms(uri, fileName, sizeBytes, preferredSubImageIndex)
        } else {
            val bitmap = decodeUri(uri)
            ViewerImage(
                bitmap = bitmap,
                fileName = fileName,
                uri = uri,
                format = format,
                sizeBytes = sizeBytes,
                width = bitmap.width,
                height = bitmap.height
            )
        }
    }

    suspend fun scanSiblingImages(uri: Uri): List<Uri> = withContext(Dispatchers.IO) {
        val documentsResult = runCatching { scanDocumentsProviderSiblings(uri) }.getOrElse { listOf(uri) }
        if (documentsResult.size > 1) return@withContext documentsResult

        val mediaStoreResult = runCatching { scanMediaStoreSiblings(uri) }.getOrElse { listOf(uri) }
        if (mediaStoreResult.size > 1) mediaStoreResult else documentsResult
    }

    suspend fun scanTreeImages(treeUri: Uri): List<Uri> = withContext(Dispatchers.IO) {
        runCatching { scanTreeChildren(treeUri) }.getOrElse { emptyList() }
    }

    suspend fun openSubImage(current: ViewerImage, subImageIndex: Int): ViewerImage = withContext(Dispatchers.IO) {
        val imsFile = requireNotNull(current.imsFile) { "Current file is not IMS" }
        val bytes = readAllBytes(current.uri)
        val target = imsFile.entries.getOrNull(subImageIndex) ?: error("Invalid sub image index")
        val imageBytes = bytes.copyOfRange(target.offset.toInt(), (target.offset + target.length).toInt())
        val bitmap = decodeBytes(imageBytes)
        val updatedEntry = target.copy(width = bitmap.width, height = bitmap.height)
        val updatedIms = imsFile.copy(
            entries = imsFile.entries.map { if (it.index == updatedEntry.index) updatedEntry else it }
        )

        current.copy(
            bitmap = bitmap,
            width = bitmap.width,
            height = bitmap.height,
            imsFile = updatedIms,
            subImageIndex = subImageIndex
        )
    }

    suspend fun makeThumbnail(current: ViewerImage, subImage: SubImage): Bitmap = withContext(Dispatchers.IO) {
        val bytes = readAllBytes(current.uri)
        val imageBytes = bytes.copyOfRange(subImage.offset.toInt(), (subImage.offset + subImage.length).toInt())
        decodeBytes(imageBytes, targetMaxSize = 180)
    }

    private fun openIms(uri: Uri, fileName: String, sizeBytes: Long, preferredSubImageIndex: Int): ViewerImage {
        val bytes = readAllBytes(uri)
        val imsFile = imsParser.parse(bytes)
        val coverIndex = imsFile.entries.indexOfFirst { it.isCover }.takeIf { it >= 0 } ?: 0
        val subImageIndex = preferredSubImageIndex.coerceIn(0, imsFile.entries.lastIndex).takeIf {
            preferredSubImageIndex in imsFile.entries.indices
        } ?: coverIndex
        val entry = imsFile.entries[subImageIndex]
        val imageBytes = bytes.copyOfRange(entry.offset.toInt(), (entry.offset + entry.length).toInt())
        val bitmap = decodeBytes(imageBytes)
        val updatedEntry = entry.copy(width = bitmap.width, height = bitmap.height)
        val updatedIms = imsFile.copy(
            entries = imsFile.entries.map { if (it.index == updatedEntry.index) updatedEntry else it }
        )

        return ViewerImage(
            bitmap = bitmap,
            fileName = fileName,
            uri = uri,
            format = ImageFormat.Ims,
            sizeBytes = sizeBytes.takeIf { it >= 0 } ?: bytes.size.toLong(),
            width = bitmap.width,
            height = bitmap.height,
            imsFile = updatedIms,
            subImageIndex = subImageIndex
        )
    }

    private fun scanDocumentsProviderSiblings(uri: Uri): List<Uri> {
        if (!DocumentsContract.isDocumentUri(context, uri)) return listOf(uri)

        val documentId = DocumentsContract.getDocumentId(uri)
        val splitIndex = documentId.lastIndexOf('/')
        if (splitIndex < 0) return listOf(uri)

        val parentDocumentId = documentId.substring(0, splitIndex)
        val treeUri = DocumentsContract.buildTreeDocumentUri(uri.authority, parentDocumentId)
        val childrenUri = DocumentsContract.buildChildDocumentsUri(uri.authority, parentDocumentId)
        val result = mutableListOf<Pair<String, Uri>>()

        resolver.query(
            childrenUri,
            arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE
            ),
            null,
            null,
            null
        )?.use { cursor ->
            val idColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val nameColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            val mimeColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_MIME_TYPE)
            while (cursor.moveToNext()) {
                val childId = cursor.getString(idColumn)
                val name = cursor.getString(nameColumn) ?: continue
                val mime = cursor.getString(mimeColumn)
                if (isSupportedFileName(name, mime)) {
                    result += name to DocumentsContract.buildDocumentUriUsingTree(treeUri, childId)
                }
            }
        }

        return result
            .sortedWith(compareBy<Pair<String, Uri>> { it.first.lowercase(Locale.US) }.thenBy { it.first })
            .map { it.second }
            .ifEmpty { listOf(uri) }
    }

    private fun scanTreeChildren(treeUri: Uri): List<Uri> {
        val treeDocumentId = DocumentsContract.getTreeDocumentId(treeUri)
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, treeDocumentId)
        val result = mutableListOf<Pair<String, Uri>>()

        resolver.query(
            childrenUri,
            arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE
            ),
            null,
            null,
            null
        )?.use { cursor ->
            val idColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val nameColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            val mimeColumn = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_MIME_TYPE)
            while (cursor.moveToNext()) {
                val childId = cursor.getString(idColumn)
                val name = cursor.getString(nameColumn) ?: continue
                val mime = cursor.getString(mimeColumn)
                if (isSupportedFileName(name, mime)) {
                    result += name to DocumentsContract.buildDocumentUriUsingTree(treeUri, childId)
                }
            }
        }

        return result
            .sortedWith(compareBy<Pair<String, Uri>> { it.first.lowercase(Locale.US) }.thenBy { it.first })
            .map { it.second }
    }

    private fun scanMediaStoreSiblings(uri: Uri): List<Uri> {
        val currentName = queryFileName(uri)
        val relativePath = queryRelativePath(uri) ?: return listOf(uri)
        val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL)
        } else {
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        }
        val projection = arrayOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.DISPLAY_NAME,
            MediaStore.Images.Media.MIME_TYPE,
            MediaStore.Images.Media.RELATIVE_PATH
        )
        val result = mutableListOf<Pair<String, Uri>>()

        resolver.query(
            collection,
            projection,
            "${MediaStore.Images.Media.RELATIVE_PATH}=?",
            arrayOf(relativePath),
            null
        )?.use { cursor ->
            val idColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
            val nameColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DISPLAY_NAME)
            val mimeColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.MIME_TYPE)
            while (cursor.moveToNext()) {
                val id = cursor.getLong(idColumn)
                val name = cursor.getString(nameColumn) ?: continue
                val mime = cursor.getString(mimeColumn)
                if (isSupportedFileName(name, mime)) {
                    result += name to Uri.withAppendedPath(collection, id.toString())
                }
            }
        }

        val sorted = result
            .sortedWith(compareBy<Pair<String, Uri>> { it.first.lowercase(Locale.US) }.thenBy { it.first })
            .toMutableList()
        val currentIndex = sorted.indexOfFirst { it.first == currentName }
        if (currentIndex < 0) {
            sorted += currentName to uri
        }
        return sorted.map { it.second }.ifEmpty { listOf(uri) }
    }

    private fun queryRelativePath(uri: Uri): String? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return null
        resolver.query(uri, arrayOf(MediaStore.Images.Media.RELATIVE_PATH), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val index = cursor.getColumnIndex(MediaStore.Images.Media.RELATIVE_PATH)
                if (index >= 0 && !cursor.isNull(index)) return cursor.getString(index)
            }
        }
        return null
    }

    private fun isSupportedFileName(name: String, mime: String?): Boolean {
        val lowerName = name.lowercase(Locale.US)
        return lowerName.endsWith(".ims") ||
            lowerName.contains(".ims.") ||
            lowerName.endsWith(".jpg") ||
            lowerName.endsWith(".jpeg") ||
            lowerName.endsWith(".png") ||
            lowerName.endsWith(".webp") ||
            lowerName.endsWith(".avif") ||
            mime?.startsWith("image/") == true
    }

    private fun queryFileName(uri: Uri): String {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (index >= 0) return cursor.getString(index)
            }
        }
        return uri.lastPathSegment?.substringAfterLast('/') ?: "Untitled"
    }

    private fun querySize(uri: Uri): Long {
        resolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val index = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (index >= 0 && !cursor.isNull(index)) return cursor.getLong(index)
            }
        }
        return -1L
    }

    private fun readAllBytes(uri: Uri): ByteArray {
        return resolver.openInputStream(uri)?.use { it.readBytes() }
            ?: error("Cannot open input stream")
    }

    private fun decodeUri(uri: Uri): Bitmap {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(resolver, uri)) { decoder, _, _ ->
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            }
        } else {
            resolver.openInputStream(uri)?.use { stream ->
                BitmapFactory.decodeStream(stream)
            } ?: error("Cannot decode image")
        }
    }

    private fun decodeBytes(bytes: ByteArray, targetMaxSize: Int? = null): Bitmap {
        val options = BitmapFactory.Options()
        if (targetMaxSize != null) {
            options.inJustDecodeBounds = true
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            options.inSampleSize = calculateInSampleSize(options.outWidth, options.outHeight, targetMaxSize)
            options.inJustDecodeBounds = false
        }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            ?: error("Cannot decode image bytes")
    }

    private fun calculateInSampleSize(width: Int, height: Int, targetMaxSize: Int): Int {
        var sample = 1
        var currentWidth = width
        var currentHeight = height
        while (currentWidth / 2 >= targetMaxSize || currentHeight / 2 >= targetMaxSize) {
            sample *= 2
            currentWidth /= 2
            currentHeight /= 2
        }
        return sample.coerceAtLeast(1)
    }
}
