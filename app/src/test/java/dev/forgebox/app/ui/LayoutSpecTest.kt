package dev.forgebox.app.ui

import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LayoutSpecTest {

    @Test
    fun clampsTerminalFractionToAllowedRange() {
        assertEquals(LayoutSpec.MIN_TERMINAL_FRACTION, LayoutSpec.clampTerminalFraction(-1f))
        assertEquals(LayoutSpec.MAX_TERMINAL_FRACTION, LayoutSpec.clampTerminalFraction(9f))
        assertEquals(0.42f, LayoutSpec.clampTerminalFraction(0.42f))
    }

    @Test
    fun detectsCompactAndExpandedWidths() {
        assertFalse(LayoutSpec.isExpanded(360.dp))
        assertFalse(LayoutSpec.isExpanded(599.dp))
        assertTrue(LayoutSpec.isExpanded(600.dp))
        assertTrue(LayoutSpec.isExpanded(1280.dp))
    }
}
