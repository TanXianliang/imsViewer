package com.imsviewer.android.domain.model

import org.junit.Assert.assertEquals
import org.junit.Test

class BrowserDisplayModeTest {
    @Test
    fun restoresPersistedMode() {
        assertEquals(
            BrowserDisplayMode.SmallIcons,
            BrowserDisplayMode.fromPreference("small_icons", BrowserDisplayMode.List)
        )
    }

    @Test
    fun unknownValueFallsBackToScreenDefault() {
        assertEquals(
            BrowserDisplayMode.MediumIcons,
            BrowserDisplayMode.fromPreference("unknown", BrowserDisplayMode.MediumIcons)
        )
    }
}
