package dev.forgebox.app.terminal

import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TerminalControllerLogicTest {
    private val dispatcher = StandardTestDispatcher()
    @Before fun setUp() { Dispatchers.setMain(dispatcher) }
    @After fun tearDown() { Dispatchers.resetMain() }

    @Test
    fun diffParserParsesState() {
        val json = """{"rows":2,"cols":5,"cursor":[1,2],"lines":["hello","wo"]}""".toByteArray()
        val state = TerminalDiffParser.parse(json, TerminalRenderState())
        assertEquals(2, state.rows)
        assertEquals(5, state.cols)
        assertEquals(1, state.cursorRow)
        assertEquals(2, state.cursorCol)
        assertEquals("hello", state.lines[0])
    }

    @Test
    fun diffParserFallsBackOnBadInput() {
        val fallback = TerminalRenderState(rows = 3, cols = 3, lines = listOf("a", "b", "c"))
        val state = TerminalDiffParser.parse("not json".toByteArray(), fallback)
        assertEquals(fallback, state)
    }
}
