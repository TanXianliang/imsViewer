package com.imsviewer.android.domain.model

enum class BrowserDisplayMode(val preferenceValue: String) {
    LargeIcons("large_icons"),
    MediumIcons("medium_icons"),
    SmallIcons("small_icons"),
    List("list");

    companion object {
        fun fromPreference(value: String?, default: BrowserDisplayMode): BrowserDisplayMode {
            return entries.firstOrNull { it.preferenceValue == value } ?: default
        }
    }
}
