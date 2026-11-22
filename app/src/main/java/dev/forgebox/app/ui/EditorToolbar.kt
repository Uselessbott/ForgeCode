package dev.forgebox.app.ui

sealed class EditorToolbarAction {
    data class Insert(val text: String) : EditorToolbarAction()
    data object Tab : EditorToolbarAction()
    data object Escape : EditorToolbarAction()
    data object Delete : EditorToolbarAction()
    data object Home : EditorToolbarAction()
    data object End : EditorToolbarAction()
    data object Left : EditorToolbarAction()
    data object Right : EditorToolbarAction()
    data object Up : EditorToolbarAction()
    data object Down : EditorToolbarAction()
    data object Ctrl : EditorToolbarAction()
    data object Alt : EditorToolbarAction()
    data object Shift : EditorToolbarAction()
}

object EditorToolbarKeyMapper {
    val keys: List<String> = listOf(
        "ESC", "TAB", "CTRL", "ALT", "SHIFT", "HOME", "END", "DEL",
        "←", "↓", "↑", "→", "(", ")", "[", "]", "{", "}", "\"", "'",
        ":", ";", "=", "+", "-", "*", "/", "\\", "|", "&", "$", "#", "@", "!", "?", "<", ">",
    )

    fun map(key: String): EditorToolbarAction = when (key) {
        "ESC" -> EditorToolbarAction.Escape
        "TAB" -> EditorToolbarAction.Tab
        "CTRL" -> EditorToolbarAction.Ctrl
        "ALT" -> EditorToolbarAction.Alt
        "SHIFT" -> EditorToolbarAction.Shift
        "HOME" -> EditorToolbarAction.Home
        "END" -> EditorToolbarAction.End
        "DEL" -> EditorToolbarAction.Delete
        "←" -> EditorToolbarAction.Left
        "↓" -> EditorToolbarAction.Down
        "↑" -> EditorToolbarAction.Up
        "→" -> EditorToolbarAction.Right
        else -> EditorToolbarAction.Insert(key)
    }
}
