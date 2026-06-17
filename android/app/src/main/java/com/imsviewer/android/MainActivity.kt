package com.imsviewer.android

import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.Window
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.core.content.ContextCompat
import androidx.activity.viewModels
import com.imsviewer.android.ui.MainScreen
import com.imsviewer.android.ui.theme.ImsViewerTheme
import com.imsviewer.android.viewmodel.ViewerViewModel

class MainActivity : ComponentActivity() {
    private val viewModel: ViewerViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.statusBarColor = Color.rgb(17, 17, 20)
        window.navigationBarColor = Color.rgb(17, 17, 20)
        requestMediaPermissionIfNeeded()
        handleIncomingIntent(intent)

        setContent {
            ImsViewerTheme {
                val openLauncher = rememberLauncherForActivityResult(
                    contract = ActivityResultContracts.OpenDocument()
                ) { uri: Uri? ->
                    if (uri != null) {
                        viewModel.openUri(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    }
                }
                val openFolderLauncher = rememberLauncherForActivityResult(
                    contract = ActivityResultContracts.OpenDocumentTree()
                ) { uri: Uri? ->
                    if (uri != null) {
                        viewModel.openFolder(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    }
                }

                MainScreen(
                    viewModel = viewModel,
                    onOpenFile = {
                        openLauncher.launch(
                            arrayOf(
                                "image/*",
                                "application/octet-stream"
                            )
                        )
                    },
                    onOpenFolder = {
                        openFolderLauncher.launch(null)
                    },
                    onShare = { intent ->
                        startActivity(Intent.createChooser(intent, "Share image"))
                    }
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIncomingIntent(intent)
    }

    private fun handleIncomingIntent(intent: Intent?) {
        if (intent == null) return
        val uris = extractIncomingUris(intent)
        if (uris.isEmpty()) return

        if (uris.size > 1) {
            viewModel.openUriSequence(uris, intent.flags)
        } else {
            viewModel.openUri(uris.first(), intent.flags)
        }
    }

    private fun extractIncomingUris(intent: Intent): List<Uri> {
        val result = linkedSetOf<Uri>()
        intent.data?.let(result::add)

        intent.clipData?.let { clipData ->
            repeat(clipData.itemCount) { index ->
                clipData.getItemAt(index).uri?.let(result::add)
            }
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)?.let(result::add)
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)?.let(result::addAll)
        } else {
            @Suppress("DEPRECATION")
            (intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let(result::add)
            @Suppress("DEPRECATION")
            (intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM))?.let(result::addAll)
        }

        return result.toList()
    }

    private fun requestMediaPermissionIfNeeded() {
        val permission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            android.Manifest.permission.READ_MEDIA_IMAGES
        } else {
            android.Manifest.permission.READ_EXTERNAL_STORAGE
        }
        if (ContextCompat.checkSelfPermission(this, permission) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(permission), 10)
        }
    }
}
