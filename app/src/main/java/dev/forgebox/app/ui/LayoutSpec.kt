package dev.forgebox.app.ui

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Pure layout policy so it can be unit-tested without an emulator. */
object LayoutSpec {
    const val MIN_TERMINAL_FRACTION: Float = 0.20f
    const val MAX_TERMINAL_FRACTION: Float = 0.70f
    const val DEFAULT_TERMINAL_FRACTION: Float = 0.40f

    val CompactWidthBreakpoint: Dp = 600.dp

    fun clampTerminalFraction(value: Float): Float =
        value.coerceIn(MIN_TERMINAL_FRACTION, MAX_TERMINAL_FRACTION)

    fun isExpanded(width: Dp): Boolean = width >= CompactWidthBreakpoint
}
