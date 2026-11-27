package dev.forgebox.app.terminal

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
fun TerminalPanel(
    state: TerminalUiState,
    onCreate: () -> Unit,
    onSelect: (Long) -> Unit,
    onRename: (Long, String) -> Unit,
    onWrite: (String) -> Unit,
    onWriteBytes: (ByteArray) -> Unit,
    onSignal: (Int) -> Unit,
    onStop: () -> Unit,
    onStopAll: () -> Unit,
    onCloseSession: (Long) -> Unit,
    onRecreate: (Long) -> Unit,
    onDismissRestore: () -> Unit,
    onApproveApproval: () -> Unit = {},
    onDenyApproval: () -> Unit = {},
    onShellKindChange: (TerminalShellKind) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val focusRequester = remember { FocusRequester() }
    var input by remember { mutableStateOf("") }
    var renameId by remember { mutableStateOf<Long?>(null) }
    var renameText by remember { mutableStateOf("") }
    var confirmStopAll by remember { mutableStateOf(false) }
    var selectionStart by remember { mutableStateOf<Int?>(null) }
    var selectionEnd by remember { mutableStateOf<Int?>(null) }
    val measurer = rememberTextMeasurer()
    val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
    val active = state.sessions.firstOrNull { it.ptyId == state.activePtyId }
    val canCreate = state.sessions.count { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING } < state.sessionCap

    Column(modifier.background(Color(0xFF111111))) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            TextButton(onClick = onCreate, enabled = canCreate) { Text("+") }
            TextButton(onClick = { onShellKindChange(TerminalShellKind.BIONIC) }) { Text(if (state.preferredShellKind==TerminalShellKind.BIONIC) "[Bionic]" else "Bionic", color = Color.White) }
            TextButton(onClick = { if (state.linuxEnvReady) onShellKindChange(TerminalShellKind.LINUX) }, enabled = state.linuxEnvReady) { Text(if (state.preferredShellKind==TerminalShellKind.LINUX) "[Linux]" else "Linux", color = if (state.linuxEnvReady) Color.White else Color.Gray) }
            if (!canCreate) Text("cap ${state.sessionCap}", color = Color(0xFFCCAA66), style = MaterialTheme.typography.labelSmall)
            Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                state.sessions.forEach { s ->
                    val selected = s.ptyId == state.activePtyId
                    TextButton(onClick = { onSelect(s.ptyId) }) {
                        val k = if (s.shellKind == TerminalShellKind.LINUX) "L" else "B"
                        Text(
                            "[$k] ${s.name}" + if (s.lifecycle == TerminalSessionLifecycle.EXITED) " [exit]" else if (s.restorable) " [restore]" else "",
                            color = if (selected) Color.White else Color(0xFFBBBBBB),
                        )
                    }
                    TextButton(onClick = { renameId = s.ptyId; renameText = s.name }) { Text("âœŽ", color = Color(0xFFAAAAAA)) }
                    TextButton(onClick = { onCloseSession(s.ptyId) }) { Text("Ã—", color = Color(0xFFAAAAAA)) }
                }
            }
            TextButton(onClick = onStop) { Text("Stop") }
            TextButton(onClick = { confirmStopAll = true }) { Text("Stop All") }
        }

        state.restoreBanner?.let { banner ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(banner, color = Color(0xFFFFCC66), style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                active?.takeIf { it.restorable }?.let { s -> TextButton(onClick = { onRecreate(s.ptyId) }) { Text("Recreate") } }
                TextButton(onClick = onDismissRestore) { Text("Dismiss") }
            }
        }
        active?.scrollbackNotice?.let {
            Text(it, color = Color(0xFFFFAA55), style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(horizontal = 8.dp))
        }
        if (state.hintShown) {
            val msg = if (state.linuxEnvReady || state.sessions.any { it.shellKind == TerminalShellKind.LINUX }) {
                "Linux shell: bundled Alpine minirootfs via PRoot (compatibility layer, not a security sandbox). Same app UID; no separate Linux user isolation. Offline bootstrap in M10.1."
            } else {
                "M7 terminal: real /system/bin/sh + toybox. Multiple sessions, no persistence across app kill (restore recreates shell). No python/rustc/node/gcc, no apt, no PRoot/distro."
            }
            Text(msg, color = Color(0xFFCCAA66), style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(horizontal = 8.dp))
        }

        if (active == null) {
            Box(Modifier.fillMaxWidth().weight(1f), contentAlignment = androidx.compose.ui.Alignment.Center) {
                Text("No terminal session. Press + to create one.", color = Color(0xFFBBBBBB))
            }
        } else {
            Canvas(
                Modifier.fillMaxWidth().weight(1f).pointerInput(active.ptyId, active.render) {
                    detectTapGestures(
                        onTap = { focusRequester.requestFocus(); selectionStart = null; selectionEnd = null },
                        onLongPress = { offset ->
                            val row = (offset.y / (18.dp.toPx())).toInt().coerceIn(0, (active.render.lines.size - 1).coerceAtLeast(0))
                            selectionStart = row; selectionEnd = row
                        },
                    )
                }
            ) {
                drawRect(Color(0xFF111111))
                val mapped = TerminalStyleMapper.map(active.render.fg, active.render.bg, active.render.bold, active.render.underline, active.render.inverse)
                drawRect(Color(mapped.backgroundArgb))
                val style = TextStyle(
                    color = Color(mapped.colorArgb),
                    fontFamily = FontFamily.Monospace,
                    fontSize = 13.sp,
                    fontWeight = if (mapped.bold) androidx.compose.ui.text.font.FontWeight.Bold else null,
                    textDecoration = if (mapped.underline) androidx.compose.ui.text.style.TextDecoration.Underline else null,
                )
                val lineHeight = 18.dp.toPx()
                active.render.lines.forEachIndexed { i, line ->
                    if (selectionStart != null && selectionEnd != null) {
                        val copied = TerminalSelectionMapper.selectedText(active.render.lines, selectionStart!!, selectionEnd!!)
                        val s = minOf(selectionStart!!, selectionEnd!!)
                        val e = maxOf(selectionStart!!, selectionEnd!!)
                        if (i in s..e) drawRect(Color(0x44FFFFFF), topLeft = Offset(0f, i * lineHeight), size = androidx.compose.ui.geometry.Size(size.width, lineHeight))
                        if (copied.isNotEmpty()) { /* selection active */ }
                    }
                    val layout = measurer.measure(line.ifEmpty { " " }, style)
                    drawText(layout, topLeft = Offset(4f, i * lineHeight))
                }
                val cx = 4f + active.render.cursorCol * 8.dp.toPx() * 0.6f
                val cy = active.render.cursorRow * lineHeight
                drawRect(Color(0xAAFFFFFF), topLeft = Offset(cx, cy), size = androidx.compose.ui.geometry.Size(8f, lineHeight))
            }
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(active.statusLabel(), color = Color(0xFFAAAAAA), style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                if (selectionStart != null && selectionEnd != null) {
                    TextButton(onClick = {
                        val copied = TerminalSelectionMapper.selectedText(active.render.lines, selectionStart!!, selectionEnd!!)
                        clipboard.setText(androidx.compose.ui.text.AnnotatedString(copied))
                        selectionStart = null; selectionEnd = null
                    }) { Text("Copy sel") }
                }
            }
        }

        Row(Modifier.fillMaxWidth().padding(4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            listOf("ESC" to byteArrayOf(27), "TAB" to byteArrayOf(9), "CTRL+C" to byteArrayOf(3), "CTRL+D" to byteArrayOf(4), "CTRL+Z" to byteArrayOf(26)).forEach { (label, bytes) ->
                TextButton(onClick = { onWriteBytes(bytes) }) { Text(label, color = Color.White) }
            }
        }

        BasicTextField(
            value = input,
            onValueChange = { new ->
                if (new.length > input.length) onWrite(new.substring(input.length))
                else if (new.length < input.length) onWriteBytes(byteArrayOf(0x7f))
                input = ""
            },
            modifier = Modifier.fillMaxWidth().height(1.dp).focusRequester(focusRequester),
            textStyle = TextStyle(color = Color.Transparent),
            cursorBrush = androidx.compose.ui.graphics.SolidColor(Color.Transparent),
        )
    }

    if (renameId != null) {
        AlertDialog(
            onDismissRequest = { renameId = null },
            title = { Text("Rename session") },
            text = { OutlinedTextField(value = renameText, onValueChange = { renameText = it }, label = { Text("Name") }) },
            confirmButton = { TextButton(onClick = { onRename(renameId!!, renameText); renameId = null }) { Text("Rename") } },
            dismissButton = { TextButton(onClick = { renameId = null }) { Text("Cancel") } },
        )
    }
    if (confirmStopAll) {
        AlertDialog(
            onDismissRequest = { confirmStopAll = false },
            title = { Text("Stop all sessions?") },
            text = { Text("This sends stop to all running terminal sessions.") },
            confirmButton = { TextButton(onClick = { confirmStopAll = false; onStopAll() }) { Text("Stop All") } },
            dismissButton = { TextButton(onClick = { confirmStopAll = false }) { Text("Cancel") } },
        )
    }
    state.pendingApproval?.let { p ->
        AlertDialog(
            onDismissRequest = onDenyApproval,
            title = { Text("Confirm command") },
            text = {
                Column {
                    Text(p.reason)
                    Text("Command: ${p.command}", style = MaterialTheme.typography.bodySmall)
                    Text("cwd: ${p.cwd}", style = MaterialTheme.typography.bodySmall)
                    Text("Affects outside project: ${p.affectsOutsideProject}; network: ${p.usesNetwork}; checkpoint before execution: ${p.createsCheckpoint}", style = MaterialTheme.typography.bodySmall)
                }
            },
            confirmButton = { TextButton(onClick = onApproveApproval) { Text("Approve once") } },
            dismissButton = { TextButton(onClick = onDenyApproval) { Text("Deny") } },
        )
    }
}

private fun TerminalSessionUi.statusLabel(): String = when (lifecycle) {
    TerminalSessionLifecycle.STARTING -> "starting"
    TerminalSessionLifecycle.RUNNING -> "running"
    TerminalSessionLifecycle.EXITED -> "exited (${exitCode ?: 0})"
    TerminalSessionLifecycle.STOPPED -> "stopped"
    TerminalSessionLifecycle.CRASHED -> "crashed (${exitCode ?: -1})"
    TerminalSessionLifecycle.RESTORABLE -> "previous process ended"
}
