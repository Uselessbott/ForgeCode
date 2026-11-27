package dev.forgebox.app.ui

import android.view.ViewGroup
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.ScrollableTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import dev.forgebox.app.service.SessionService
import dev.forgebox.app.terminal.TerminalPanel
import dev.forgebox.app.terminal.TerminalSessionController
import dev.forgebox.app.terminal.TerminalSessionLifecycle
import io.github.rosemoe.sora.event.ContentChangeEvent
import io.github.rosemoe.sora.lang.EmptyLanguage
import io.github.rosemoe.sora.langs.textmate.TextMateColorScheme
import io.github.rosemoe.sora.langs.textmate.TextMateLanguage
import io.github.rosemoe.sora.langs.textmate.registry.FileProviderRegistry
import io.github.rosemoe.sora.langs.textmate.registry.GrammarRegistry
import io.github.rosemoe.sora.langs.textmate.registry.ThemeRegistry
import io.github.rosemoe.sora.langs.textmate.registry.model.ThemeModel
import io.github.rosemoe.sora.langs.textmate.registry.provider.AssetsFileResolver
import io.github.rosemoe.sora.widget.CodeEditor
import org.eclipse.tm4e.core.registry.IThemeSource
import kotlinx.coroutines.delay

@Composable
fun EditorScreen(
    projectId: String,
    initialPath: String,
    viewModel: EditorViewModel,
    onBack: () -> Unit,
    initialLine: Int = 1,
    initialColumn: Int = 1,
) {
    val state by viewModel.state.collectAsState()
    var pendingClose by remember { mutableStateOf<Int?>(null) }
    var showFind by remember { mutableStateOf(false) }
    var showGoTo by remember { mutableStateOf(false) }
    var findQuery by remember { mutableStateOf("") }
    var replaceQuery by remember { mutableStateOf("") }
    var goToLine by remember { mutableStateOf("") }
    val editorRef = remember { mutableStateOf<CodeEditor?>(null) }
    val terminalState by TerminalSessionController.state.collectAsState()
    var terminalFraction by remember { mutableStateOf(0.4f) }
    val context = LocalContext.current

    LaunchedEffect(projectId, initialPath, initialLine, initialColumn) { viewModel.openFileAt(projectId, initialPath, initialLine, initialColumn) }
    LaunchedEffect(terminalState.open) {
        if (terminalState.open) SessionService.open(context, projectId)
    }
    val hasRunningTerminal = terminalState.sessions.any { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING }
    LaunchedEffect(terminalState.open, hasRunningTerminal, state.activeIndex) {
        while (terminalState.open && hasRunningTerminal) {
            viewModel.checkActiveDiskChange()
            delay(2000)
        }
    }

    Scaffold { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Files") }
                Text(projectId, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                TextButton(onClick = { showGoTo = true }) { Text("Go to line") }
                TextButton(onClick = { showFind = true }) { Text("Find") }
                TextButton(onClick = { TerminalSessionController.toggle(context, projectId) }) { Text(if (terminalState.open) "Hide Terminal" else "Terminal") }
                TextButton(onClick = { viewModel.saveActive() }) { Text("Save") }
            }

            if (state.tabs.isNotEmpty()) {
                ScrollableTabRow(selectedTabIndex = state.activeIndex.coerceAtLeast(0), edgePadding = 0.dp) {
                    state.tabs.forEachIndexed { index, tab ->
                        Tab(
                            selected = index == state.activeIndex,
                            onClick = { viewModel.selectTab(index) },
                            text = {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Text((if (tab.dirty) "● " else "") + tab.path.substringAfterLast('/'))
                                    Spacer(Modifier.width(4.dp))
                                    TextButton(onClick = {
                                        if (!viewModel.closeTab(index)) pendingClose = index
                                    }) { Text("x") }
                                }
                            },
                        )
                    }
                }
            }

            state.actionError?.let {
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 8.dp))
            }

            val tab = state.tabs.getOrNull(state.activeIndex)
            val editorWeight = if (terminalState.open) 1f - terminalFraction else 1f
            Column(Modifier.weight(editorWeight).fillMaxWidth()) {
                when {
                    tab == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("No open file") }
                    tab.loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                    tab.error != null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(tab.error ?: "Error", color = MaterialTheme.colorScheme.error)
                            if (tab.tooLarge || tab.readOnly) Text("M5 guard: file not editable in editor.", style = MaterialTheme.typography.bodySmall)
                            Button(onClick = { viewModel.reloadActive() }) { Text("Reload") }
                        }
                    }
                    else -> {
                        tab.conflict?.let {
                            Text("Conflict: $it", color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(8.dp))
                            Row(Modifier.padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Button(onClick = { viewModel.reloadActive() }) { Text("Reload disk") }
                                Button(onClick = { viewModel.overwriteActive() }) { Text("Overwrite disk") }
                            }
                        }
                        SoraEditorView(
                            text = tab.content,
                            path = tab.path,
                            readOnly = tab.readOnly,
                            onTextChanged = { viewModel.onContentChanged(state.activeIndex, it) },
                            editorRef = editorRef,
                            modifier = Modifier.weight(1f).fillMaxWidth(),
                        )
                    }
                }
            }

            if (terminalState.open) {
                Box(
                    Modifier.fillMaxWidth().height(10.dp).pointerInput(Unit) {
                        detectDragGestures { _, drag -> terminalFraction = (terminalFraction - drag.y / 1000f).coerceIn(0.2f, 0.7f) }
                    },
                    contentAlignment = Alignment.Center,
                ) { HorizontalDivider() }
                TerminalPanel(
                    state = terminalState,
                    onCreate = { TerminalSessionController.createSession(projectId) },
                    onSelect = TerminalSessionController::select,
                    onRename = TerminalSessionController::rename,
                    onWrite = TerminalSessionController::write,
                    onWriteBytes = TerminalSessionController::writeBytes,
                    onSignal = TerminalSessionController::signal,
                    onStop = TerminalSessionController::stop,
                    onStopAll = TerminalSessionController::stopAll,
                    onCloseSession = TerminalSessionController::closeSession,
                    onRecreate = TerminalSessionController::recreateRestorable,
                    onDismissRestore = TerminalSessionController::dismissRestoreBanner,
                    onApproveApproval = TerminalSessionController::approvePending,
                    onDenyApproval = TerminalSessionController::denyPending,
                    onShellKindChange = TerminalSessionController::setPreferredShellKind,
                    modifier = Modifier.fillMaxWidth().weight(terminalFraction),
                )
            }

            ProgrammingToolbar(onAction = { action -> applyToolbarAction(editorRef.value, action) })
        }
    }

    if (showFind) {
        AlertDialog(
            onDismissRequest = { showFind = false },
            title = { Text("Find / Replace") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(value = findQuery, onValueChange = { findQuery = it }, label = { Text("Find") })
                    OutlinedTextField(value = replaceQuery, onValueChange = { replaceQuery = it }, label = { Text("Replace") })
                    Text("In-file search uses editor search where available. Find-in-files across project is deferred/indexed later.", style = MaterialTheme.typography.bodySmall)
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    val editor = editorRef.value
                    if (editor != null && findQuery.isNotEmpty()) {
                        val text = editor.text.toString()
                        val idx = text.indexOf(findQuery, startIndex = editor.cursor.right)
                        val found = if (idx >= 0) idx else text.indexOf(findQuery)
                        if (found >= 0) editor.setSelection(found, found + findQuery.length)
                    }
                    showFind = false
                }) { Text("Find next") }
            },
            dismissButton = {
                Row {
                    TextButton(onClick = {
                        val editor = editorRef.value
                        if (editor != null && findQuery.isNotEmpty()) {
                            val replaced = editor.text.toString().replace(findQuery, replaceQuery)
                            if (replaced != editor.text.toString()) {
                                editor.setText(replaced)
                                viewModel.onContentChanged(state.activeIndex, replaced)
                            }
                        }
                        showFind = false
                    }) { Text("Replace all") }
                    TextButton(onClick = { showFind = false }) { Text("Close") }
                }
            },
        )
    }

    if (showGoTo) {
        AlertDialog(
            onDismissRequest = { showGoTo = false },
            title = { Text("Go to line") },
            text = { OutlinedTextField(value = goToLine, onValueChange = { goToLine = it }, label = { Text("Line number (1-based)") }) },
            confirmButton = {
                TextButton(onClick = {
                    val line = goToLine.toIntOrNull()?.minus(1)?.coerceAtLeast(0)
                    val editor = editorRef.value
                    if (line != null && editor != null) {
                        val target = line.coerceAtMost((editor.lineCount - 1).coerceAtLeast(0))
                        runCatching { editor.setSelection(target, 0) }
                    }
                    showGoTo = false
                }) { Text("Go") }
            },
            dismissButton = { TextButton(onClick = { showGoTo = false }) { Text("Cancel") } },
        )
    }

    pendingClose?.let { idx ->
        ConfirmDialog(
            title = "Unsaved changes",
            body = "Close tab with unsaved changes?",
            confirmLabel = "Close",
            onDismiss = { pendingClose = null },
            onConfirm = { viewModel.closeTab(idx, force = true); pendingClose = null },
        )
    }
}

internal object EditorTextMate {
    @Volatile private var initialized = false
    private val lock = Any()

    fun ensureInitialized(context: android.content.Context) {
        if (initialized) return
        synchronized(lock) {
            if (initialized) return
            runCatching {
                FileProviderRegistry.getInstance().addFileProvider(AssetsFileResolver(context.applicationContext.assets))
                val themePath = "textmate/themes/darcula.json"
                val themeRegistry = ThemeRegistry.getInstance()
                themeRegistry.loadTheme(ThemeModel(IThemeSource.fromInputStream(context.applicationContext.assets.open(themePath), themePath, null), "darcula").apply { isDark = true })
                themeRegistry.setTheme("darcula")
                GrammarRegistry.getInstance().loadGrammars("textmate/languages.json")
                initialized = true
            }
        }
    }

    fun languageForPath(context: android.content.Context, path: String?): io.github.rosemoe.sora.lang.Language {
        ensureInitialized(context)
        val ext = path?.substringAfterLast('.', "")?.lowercase().orEmpty()
        val scope = when (ext) {
            "py" -> "source.python"
            "js", "jsx", "mjs", "cjs" -> "source.js"
            "json" -> "source.json"
            "md", "markdown" -> "text.markdown"
            "java" -> "source.java"
            "kt", "kts" -> "source.kotlin"
            "rs" -> "source.rust"
            "c", "cc", "cpp", "cxx", "h", "hpp" -> "source.cpp"
            else -> null
        } ?: return EmptyLanguage()
        return runCatching {
            TextMateLanguage.create(scope, true).also { it.isAutoCompleteEnabled = false }
        }.getOrElse { EmptyLanguage() }
    }

    fun applyTheme(editor: CodeEditor) {
        runCatching { editor.colorScheme = TextMateColorScheme.create(ThemeRegistry.getInstance()) }
    }
}

@Composable
internal fun SoraEditorView(
    text: String,
    path: String?,
    readOnly: Boolean,
    onTextChanged: (String) -> Unit,
    editorRef: androidx.compose.runtime.MutableState<CodeEditor?>,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val editor = remember {
        CodeEditor(context).apply {
            layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
            isWordwrap = false
            nonPrintablePaintingFlags = CodeEditor.FLAG_DRAW_WHITESPACE_LEADING or CodeEditor.FLAG_DRAW_LINE_SEPARATOR
        }
    }
    DisposableEffect(editor) {
        editorRef.value = editor
        val sub = editor.subscribeEvent(ContentChangeEvent::class.java) { _, _ -> onTextChanged(editor.text.toString()) }
        onDispose { sub.unsubscribe() }
    }
    LaunchedEffect(path) {
        EditorTextMate.applyTheme(editor)
        editor.setEditorLanguage(EditorTextMate.languageForPath(context, path))
    }
    LaunchedEffect(text, readOnly) {
        if (editor.text.toString() != text) editor.setText(text)
        editor.isEditable = !readOnly
    }
    AndroidView(factory = { editor }, modifier = modifier)
}

@Composable
private fun ProgrammingToolbar(onAction: (EditorToolbarAction) -> Unit) {
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        EditorToolbarKeyMapper.keys.forEach { k ->
            TextButton(onClick = { onAction(EditorToolbarKeyMapper.map(k)) }) { Text(k) }
        }
    }
}

fun applyToolbarAction(editor: CodeEditor?, action: EditorToolbarAction) {
    val ed = editor ?: return
    val cursor = ed.cursor
    when (action) {
        is EditorToolbarAction.Insert -> ed.insertText(action.text, action.text.length)
        EditorToolbarAction.Tab -> ed.indentOrCommitTab()
        EditorToolbarAction.Escape -> ed.setSelection(cursor.leftLine, cursor.leftColumn)
        EditorToolbarAction.Delete -> ed.deleteText()
        EditorToolbarAction.Home -> ed.setSelection(cursor.leftLine, 0)
        EditorToolbarAction.End -> {
            val col = ed.text.getColumnCount(cursor.leftLine)
            ed.setSelection(cursor.leftLine, col)
        }
        EditorToolbarAction.Left -> {
            val line = cursor.leftLine
            val col = cursor.leftColumn
            if (col > 0) ed.setSelection(line, col - 1)
            else if (line > 0) ed.setSelection(line - 1, ed.text.getColumnCount(line - 1))
        }
        EditorToolbarAction.Right -> {
            val line = cursor.rightLine
            val col = cursor.rightColumn
            val maxCol = ed.text.getColumnCount(line)
            if (col < maxCol) ed.setSelection(line, col + 1)
            else if (line < ed.lineCount - 1) ed.setSelection(line + 1, 0)
        }
        EditorToolbarAction.Up -> {
            val line = (cursor.leftLine - 1).coerceAtLeast(0)
            val col = cursor.leftColumn.coerceAtMost(ed.text.getColumnCount(line))
            ed.setSelection(line, col)
        }
        EditorToolbarAction.Down -> {
            val line = (cursor.leftLine + 1).coerceAtMost((ed.lineCount - 1).coerceAtLeast(0))
            val col = cursor.leftColumn.coerceAtMost(ed.text.getColumnCount(line))
            ed.setSelection(line, col)
        }
        EditorToolbarAction.Ctrl, EditorToolbarAction.Alt, EditorToolbarAction.Shift -> Unit
    }
    ed.requestFocus()
}
