package dev.forgebox.app.terminal

import org.junit.Assert.*
import org.junit.Test

class TerminalStyleMapperTest {
    @Test fun mapsBoldUnderline() {
        val s = TerminalStyleMapper.map(0xFF112233.toInt(), 0xFF000000.toInt(), bold = true, underline = true, inverse = false)
        assertTrue(s.bold)
        assertTrue(s.underline)
        assertEquals(0xFF112233.toInt(), s.colorArgb)
    }
    @Test fun inverseSwapsFgBg() {
        val s = TerminalStyleMapper.map(0xFFFFFFFF.toInt(), 0xFF000000.toInt(), bold = false, underline = false, inverse = true)
        assertEquals(0xFF000000.toInt(), s.colorArgb)
        assertEquals(0xFFFFFFFF.toInt(), s.backgroundArgb)
    }
}
