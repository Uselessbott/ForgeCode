package dev.forgebox.app.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.forgebox.data.FileEntryModel

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ProjectExplorerScreen(
    projectId: String,
    viewModel: ProjectExplorerViewModel,
    onBack: () -> Unit,
    onOpenFile: (String) -> Unit,
    onOpenGit: () -> Unit = {},
) {
    val state by viewModel.state.collectAsState()
    var showCreateFile by remember { mutableStateOf(false) }
    var showCreateFolder by remember { mutableStateOf(false) }
    var actionTarget by remember { mutableStateOf<FileEntryModel?>(null) }
    var renameTarget by remember { mutableStateOf<FileEntryModel?>(null) }
    var deleteTarget by remember { mutableStateOf<FileEntryModel?>(null) }
    val clipboard = LocalClipboardManager.current

    LaunchedEffect(projectId) { viewModel.openProject(projectId) }

    Scaffold(
        floatingActionButton = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                FloatingActionButton(onClick = { showCreateFolder = true }) { Text("D") }
                FloatingActionButton(onClick = { showCreateFile = true }) { Text("F") }
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back") }
                Spacer(Modifier.width(8.dp))
                Column(Modifier.weight(1f)) {
                    Text(projectId, style = MaterialTheme.typography.titleLarge)
                    Text("Project-relative paths only. Rust VFS validates all operations.", style = MaterialTheme.typography.labelSmall)
                }
                TextButton(onClick = onOpenGit) { Text("Git") }
                TextButton(onClick = { viewModel.refresh() }) { Text("Refresh") }
            }

            state.actionError?.let {
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }

            when {
                state.loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                state.error != null -> ErrorBlock(state.error!!, onRetry = viewModel::refresh)
                else -> {
                    if (state.entries.isEmpty()) {
                        Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                            Text("Empty project. Use F for file or D for folder.")
                        }
                    } else {
                        LazyColumn(Modifier.weight(1f).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            items(state.entries, key = { it.path }) { entry ->
                                val indent = (entry.path.count { it == '/' } * 12).dp
                                Card(
                                    modifier = Modifier.fillMaxWidth().padding(start = indent)
                                        .combinedClickable(
                                            onClick = { if (!entry.isDir) onOpenFile(entry.path) },
                                            onLongClick = { actionTarget = entry },
                                        ),
                                ) {
                                    Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                                        Text(if (entry.isDir) "[dir]" else "[file]", style = MaterialTheme.typography.labelSmall)
                                        Spacer(Modifier.width(8.dp))
                                        Column(Modifier.weight(1f)) {
                                            Text(entry.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                            Text(entry.path, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                        }
                                        if (!entry.isDir) Text("${entry.size}B", style = MaterialTheme.typography.labelSmall)
                                    }
                                }
                            }
                        }
                    }

                    state.selectedPath?.let { selected ->
                        Spacer(Modifier.height(8.dp))
                        Text("Preview: $selected", style = MaterialTheme.typography.titleSmall)
                        Card(Modifier.fillMaxWidth().height(180.dp)) {
                            Box(Modifier.fillMaxSize().padding(12.dp)) {
                                when {
                                    state.previewError != null -> Text(state.previewError!!, color = MaterialTheme.colorScheme.error)
                                    state.previewText != null -> LazyColumn { item { Text(state.previewText!!) } }
                                    else -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if (showCreateFile) NameDialog("Create file", "Create", "", { showCreateFile = false }) { path ->
        viewModel.createFile(path) { if (it.isSuccess) showCreateFile = false }
    }
    if (showCreateFolder) NameDialog("Create folder", "Create", "", { showCreateFolder = false }) { path ->
        viewModel.createFolder(path) { if (it.isSuccess) showCreateFolder = false }
    }

    actionTarget?.let { entry ->
        AlertDialog(
            onDismissRequest = { actionTarget = null },
            title = { Text(entry.path) },
            text = {
                Column {
                    TextButton(onClick = {
                        clipboard.setText(AnnotatedString(entry.path))
                        actionTarget = null
                    }) { Text("Copy path") }
                    if (!entry.isDir) {
                        TextButton(onClick = { onOpenFile(entry.path); actionTarget = null }) { Text("Open in editor") }
                    }
                    TextButton(onClick = { renameTarget = entry; actionTarget = null }) { Text("Rename") }
                    TextButton(onClick = { deleteTarget = entry; actionTarget = null }) {
                        Text("Delete", color = MaterialTheme.colorScheme.error)
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { actionTarget = null }) { Text("Close") } },
        )
    }

    renameTarget?.let { entry ->
        NameDialog("Rename", "Rename", entry.path, { renameTarget = null }) { newPath ->
            viewModel.rename(entry.path, newPath) { if (it.isSuccess) renameTarget = null }
        }
    }

    deleteTarget?.let { entry ->
        val recursive = entry.isDir
        ConfirmDialog(
            title = "Delete path?",
            body = buildString {
                append("Delete \"${entry.path}\"?")
                if (recursive) append(" This is a recursive folder delete.")
                append(" M13 will add checkpoints before destructive actions.")
            },
            confirmLabel = if (recursive) "Recursive delete" else "Delete",
            onDismiss = { deleteTarget = null },
            onConfirm = {
                viewModel.deletePath(entry.path, recursive) { if (it.isSuccess) deleteTarget = null }
            },
        )
    }
}

