package dev.forgebox.app.terminal

import org.junit.Assert.*
import org.junit.Test

class TerminalSelectionMapperTest {
    @Test fun selectsSingleLine() {
        assertEquals("b", TerminalSelectionMapper.selectedText(listOf("a","b","c"), 1, 1))
    }
    @Test fun selectsReversedRange() {
        assertEquals("b\nc", TerminalSelectionMapper.selectedText(listOf("a","b","c"), 2, 1))
    }
    @Test fun clampsOutOfBounds() {
        assertEquals("a\nb", TerminalSelectionMapper.selectedText(listOf("a","b"), -5, 10))
    }
}
