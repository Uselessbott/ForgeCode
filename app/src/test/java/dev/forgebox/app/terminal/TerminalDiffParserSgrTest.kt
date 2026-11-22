package dev.forgebox.app.terminal

import org.junit.Assert.*
import org.junit.Test

class TerminalDiffParserSgrTest {
    @Test fun parsesSgrAndScrollbackFields() {
        val json = """{"rows":2,"cols":5,"cursor":[0,1],"scrollTop":3,"totalLines":20,"fg":4278190335,"bg":4278190080,"bold":true,"underline":true,"inverse":false,"lines":["ab","cd"]}""".toByteArray()
        val s = TerminalDiffParser.parse(json, TerminalRenderState())
        assertEquals(3, s.scrollTop)
        assertEquals(20, s.totalLines)
        assertEquals(0xFF0000FF.toInt(), s.fg)
        assertEquals(0xFF000000.toInt(), s.bg)
        assertTrue(s.bold)
        assertTrue(s.underline)
        assertEquals(false, s.inverse)
    }
}
