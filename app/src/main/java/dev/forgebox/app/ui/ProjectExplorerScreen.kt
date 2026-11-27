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
import dev.forgebox.data.LinuxBootstrapModeUi
import dev.forgebox.data.LinuxDistroUi
import dev.forgebox.data.LinuxEnvUiState
import dev.forgebox.data.WorkspaceRepository
import dev.forgebox.net.NetworkBroker
import dev.forgebox.core.ForgeBoxRuntime

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ProjectExplorerScreen(
    projectId: String,
    viewModel: ProjectExplorerViewModel,
    onBack: () -> Unit,
    onOpenFile: (String) -> Unit,
    onOpenGit: () -> Unit = {},
    onOpenFileAt: (String, Int, Int) -> Unit = { p, _, _ -> onOpenFile(p) },
) {
    val state by viewModel.state.collectAsState()
    val context = androidx.compose.ui.platform.LocalContext.current
    val editorVm: EditorViewModel = androidx.lifecycle.viewmodel.compose.viewModel(factory = EditorViewModel.factory(viewModel.repository))
    val editorState by editorVm.state.collectAsState()
    val terminalState by dev.forgebox.app.terminal.TerminalSessionController.state.collectAsState()
    var showCreateFile by remember { mutableStateOf(false) }
    var showCreateFolder by remember { mutableStateOf(false) }
    var actionTarget by remember { mutableStateOf<FileEntryModel?>(null) }
    var renameTarget by remember { mutableStateOf<FileEntryModel?>(null) }
    var deleteTarget by remember { mutableStateOf<FileEntryModel?>(null) }
    var showLinuxEnv by remember { mutableStateOf(false) }
    var showNetwork by remember { mutableStateOf(false) }
    var showBuild by remember { mutableStateOf(false) }
    var showOverflow by remember { mutableStateOf(false) }
    var bottomPanel by remember { mutableStateOf("Files") }
    val clipboard = LocalClipboardManager.current
    val editorRef = remember { mutableStateOf<io.github.rosemoe.sora.widget.CodeEditor?>(null) }
    val linuxVm = remember {
        LinuxEnvViewModel(
            repository = WorkspaceRepository(ForgeBoxRuntime),
            runtime = ForgeBoxRuntime,
            sourcePathProvider = { distro, mode -> prepareLinuxBootstrapSource(context, distro, mode) },
        )
    }
    val linuxState by linuxVm.state.collectAsState()

    LaunchedEffect(projectId) { viewModel.openProject(projectId); linuxVm.refresh() }
    LaunchedEffect(projectId, linuxState.status?.state, linuxState.loading) {
        if (linuxState.status?.state == LinuxEnvUiState.NOT_INSTALLED && !linuxState.loading) {
            linuxVm.install(LinuxDistroUi.ALPINE, LinuxBootstrapModeUi.BUNDLED_OFFLINE)
        }
    }

    Scaffold { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("<") }
                val activeTab = editorState.tabs.getOrNull(editorState.activeIndex)
                Text(
                    activeTab?.path?.substringAfterLast('/') ?: projectId,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = { editorVm.saveActive { if (it.isSuccess) viewModel.refresh() } }, enabled = activeTab != null) { Text("Save") }
                Box {
                    TextButton(onClick = { showOverflow = true }) { Text("\u22EE") }
                    androidx.compose.material3.DropdownMenu(expanded = showOverflow, onDismissRequest = { showOverflow = false }) {
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Git") }, onClick = { showOverflow = false; onOpenGit() })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Linux") }, onClick = { showOverflow = false; showLinuxEnv = true })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Network") }, onClick = { showOverflow = false; showNetwork = true })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Build / Run") }, onClick = { showOverflow = false; showBuild = true })
                        androidx.compose.material3.DropdownMenuItem(text = { Text("Refresh files") }, onClick = { showOverflow = false; viewModel.refresh() })
                    }
                }
            }

            editorState.actionError?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 8.dp)) }

            Box(Modifier.weight(1f).fillMaxWidth()) {
                val tab = editorState.tabs.getOrNull(editorState.activeIndex)
                when {
                    tab == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Open a file from Files") }
                    tab.loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                    tab.error != null -> Box(Modifier.fillMaxSize().padding(16.dp), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(tab.error ?: "Error", color = MaterialTheme.colorScheme.error)
                            Spacer(Modifier.height(8.dp))
                            Button(onClick = { editorVm.reloadActive() }) { Text("Reload") }
                        }
                    }
                    else -> SoraEditorView(
                        text = tab.content,
                        path = tab.path,
                        readOnly = tab.readOnly || tab.tooLarge,
                        onTextChanged = { editorVm.onContentChanged(editorState.activeIndex, it) },
                        editorRef = editorRef,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }

            androidx.compose.material3.HorizontalDivider()
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("Files", "Terminal", "Output").forEach { name ->
                    TextButton(onClick = {
                        bottomPanel = if (bottomPanel == name) "" else name
                        if (name == "Terminal" && bottomPanel == "Terminal") dev.forgebox.app.terminal.TerminalSessionController.open(context, projectId)
                    }) { Text(if (bottomPanel == name) "[$name]" else name) }
                }
            }

            if (bottomPanel.isNotBlank()) {
                androidx.compose.material3.HorizontalDivider()
                Box(Modifier.fillMaxWidth().height(280.dp)) {
                    when (bottomPanel) {
                        "Files" -> WorkspaceFilesPanel(
                            state = state,
                            onRefresh = viewModel::refresh,
                            onCreateFile = { showCreateFile = true },
                            onCreateFolder = { showCreateFolder = true },
                            onOpenFile = { path -> editorVm.openFile(projectId, path); bottomPanel = "" },
                            onAction = { actionTarget = it },
                        )
                        "Terminal" -> Column(Modifier.fillMaxSize()) {
                            if (linuxState.status?.state == LinuxEnvUiState.INSTALLING || linuxState.progressMessage.isNotBlank()) {
                                Text(
                                    "Linux bootstrap: ${linuxState.progressPercent}% ${linuxState.progressMessage}",
                                    style = MaterialTheme.typography.labelSmall,
                                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                )
                            }
                            dev.forgebox.app.terminal.TerminalPanel(
                                state = terminalState,
                                onCreate = { dev.forgebox.app.terminal.TerminalSessionController.createSession(projectId) },
                                onSelect = dev.forgebox.app.terminal.TerminalSessionController::select,
                                onRename = dev.forgebox.app.terminal.TerminalSessionController::rename,
                                onWrite = dev.forgebox.app.terminal.TerminalSessionController::write,
                                onWriteBytes = dev.forgebox.app.terminal.TerminalSessionController::writeBytes,
                                onSignal = dev.forgebox.app.terminal.TerminalSessionController::signal,
                                onStop = dev.forgebox.app.terminal.TerminalSessionController::stop,
                                onStopAll = dev.forgebox.app.terminal.TerminalSessionController::stopAll,
                                onCloseSession = dev.forgebox.app.terminal.TerminalSessionController::closeSession,
                                onRecreate = dev.forgebox.app.terminal.TerminalSessionController::recreateRestorable,
                                onDismissRestore = dev.forgebox.app.terminal.TerminalSessionController::dismissRestoreBanner,
                                onApproveApproval = dev.forgebox.app.terminal.TerminalSessionController::approvePending,
                                onDenyApproval = dev.forgebox.app.terminal.TerminalSessionController::denyPending,
                                onShellKindChange = dev.forgebox.app.terminal.TerminalSessionController::setPreferredShellKind,
                                modifier = Modifier.fillMaxSize(),
                            )
                        }
                        "Output" -> WorkspaceOutputPanel(projectId = projectId, repository = viewModel.repository, onOpenDiagnostic = { p, l, c -> editorVm.openFileAt(projectId, p, l, c); bottomPanel = "" })
                    }
                }
            }
        }
    }

    if (showCreateFile) NameDialog("Create file", "Create", "", { showCreateFile = false }) { path ->
        viewModel.createFile(path) { res -> if (res.isSuccess) { showCreateFile = false; viewModel.refresh(); editorVm.openFile(projectId, path.trim().removePrefix("/")) } }
    }
    if (showCreateFolder) NameDialog("Create folder", "Create", "", { showCreateFolder = false }) { path ->
        viewModel.createFolder(path) { if (it.isSuccess) { showCreateFolder = false; viewModel.refresh() } }
    }

    actionTarget?.let { entry ->
        AlertDialog(
            onDismissRequest = { actionTarget = null },
            title = { Text(entry.path, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            text = {
                Column {
                    TextButton(onClick = { clipboard.setText(AnnotatedString(entry.path)); actionTarget = null }) { Text("Copy path") }
                    if (!entry.isDir) TextButton(onClick = { editorVm.openFile(projectId, entry.path); bottomPanel = ""; actionTarget = null }) { Text("Open in editor") }
                    TextButton(onClick = { renameTarget = entry; actionTarget = null }) { Text("Rename") }
                    TextButton(onClick = { deleteTarget = entry; actionTarget = null }) { Text("Delete", color = MaterialTheme.colorScheme.error) }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { actionTarget = null }) { Text("Close") } },
        )
    }

    renameTarget?.let { entry ->
        NameDialog("Rename", "Rename", entry.path, { renameTarget = null }) { newPath ->
            viewModel.rename(entry.path, newPath) { if (it.isSuccess) { renameTarget = null; viewModel.refresh() } }
        }
    }

    deleteTarget?.let { entry ->
        val recursive = entry.isDir
        ConfirmDialog(
            title = "Delete path?",
            body = "Delete \"${entry.path}\"?" + if (recursive) " This is a recursive folder delete." else "",
            confirmLabel = if (recursive) "Recursive delete" else "Delete",
            onDismiss = { deleteTarget = null },
            onConfirm = { viewModel.deletePath(entry.path, recursive) { if (it.isSuccess) { deleteTarget = null; viewModel.refresh() } } },
        )
    }

    if (showLinuxEnv) { LinuxEnvDialog(viewModel = linuxVm, projectId = projectId, onDismiss = { showLinuxEnv = false }) }
    if (showNetwork) {
        val netVm = remember { NetworkViewModel(NetworkBroker(WorkspaceRepository(ForgeBoxRuntime))) }
        AlertDialog(onDismissRequest = { showNetwork = false }, title = { Text("Network") }, text = { Box(Modifier.fillMaxWidth().height(520.dp)) { NetworkScreen(netVm, sessionId = projectId) } }, confirmButton = { TextButton(onClick = { showNetwork = false }) { Text("Close") } })
    }
    if (showBuild) {
        AlertDialog(onDismissRequest = { showBuild = false }, title = { Text("Build / Run") }, text = { BuildDialogContent(projectId = projectId, repository = viewModel.repository, onOpenDiagnostic = { pth: String, l: Int, c: Int -> editorVm.openFileAt(projectId, pth, l, c); showBuild = false }) }, confirmButton = { TextButton(onClick = { showBuild = false }) { Text("Close") } })
    }
}


@Composable
private fun WorkspaceFilesPanel(
    state: ProjectExplorerUiState,
    onRefresh: () -> Unit,
    onCreateFile: () -> Unit,
    onCreateFolder: () -> Unit,
    onOpenFile: (String) -> Unit,
    onAction: (FileEntryModel) -> Unit,
) {
    Column(Modifier.fillMaxSize().padding(8.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = onCreateFile) { Text("New file") }
                TextButton(onClick = onCreateFolder) { Text("New folder") }
            }
            TextButton(onClick = onRefresh) { Text("Refresh") }
        }
        state.actionError?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
        when {
            state.loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            state.error != null -> Text(state.error ?: "Error", color = MaterialTheme.colorScheme.error)
            state.entries.isEmpty() -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Empty project") }
            else -> LazyColumn(Modifier.fillMaxSize()) {
                items(state.entries, key = { it.path }) { entry ->
                    val indent = (entry.path.count { it == '/' } * 12).dp
                    Row(
                        Modifier.fillMaxWidth().padding(start = indent, top = 6.dp, bottom = 6.dp)
                            .combinedClickable(onClick = { if (!entry.isDir) onOpenFile(entry.path) }, onLongClick = { onAction(entry) }),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(if (entry.isDir) "[dir]" else "[file]", style = MaterialTheme.typography.labelSmall)
                        Spacer(Modifier.width(8.dp))
                        Text(entry.path, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    }
                }
            }
        }
    }
}

@Composable
private fun WorkspaceOutputPanel(projectId: String, repository: WorkspaceRepository, onOpenDiagnostic: (String, Int, Int) -> Unit) {
    BuildDialogContent(projectId = projectId, repository = repository, onOpenDiagnostic = onOpenDiagnostic)
}
internal fun prepareLinuxBootstrapSource(context: android.content.Context, distro: LinuxDistroUi, mode: LinuxBootstrapModeUi): Result<String> = runCatching {
    if (distro != LinuxDistroUi.ALPINE) error("Only Alpine bootstrap is supported in this build.")
    val arch = when (android.os.Build.SUPPORTED_ABIS.firstOrNull().orEmpty()) {
        "arm64-v8a", "aarch64" -> "aarch64"
        "x86_64" -> "x86_64"
        else -> error("Unsupported ABI for Alpine rootfs: ${android.os.Build.SUPPORTED_ABIS.joinToString()}")
    }
    val fileName = "alpine-minirootfs-3.20.7-$arch.tar.gz"
    val outDir = java.io.File(context.cacheDir, "forgebox-linux-bootstrap").apply { mkdirs() }
    val outFile = java.io.File(outDir, fileName)
    if (mode == LinuxBootstrapModeUi.BUNDLED_OFFLINE) {
        val assetName = "linux/$fileName"
        context.assets.open(assetName).use { input ->
            outFile.outputStream().use { output -> input.copyTo(output) }
        }
        if (!outFile.exists() || outFile.length() <= 0L) error("Failed to stage bundled rootfs: ${outFile.absolutePath}")
        return@runCatching outFile.absolutePath
    }
    // NETWORK_DOWNLOAD: fetch through M11 broker (policy/consent/allowlist) on the app side.
    val broker = NetworkBroker(WorkspaceRepository(ForgeBoxRuntime))
    val url = "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/$arch/$fileName"
    val req = okhttp3.Request.Builder().url(url).build()
    broker.execute(req, actorKind = dev.forgebox.data.NetworkActorKindUi.USER, actorId = "linux-bootstrap", sessionId = "linux-bootstrap", requireConsent = true, providerKind = "alpine-rootfs").use { resp ->
        if (!resp.isSuccessful) error("Download failed: HTTP ${resp.code}")
        val body = resp.body ?: error("Download failed: empty body")
        outFile.outputStream().use { output -> body.byteStream().copyTo(output) }
    }
    outFile.absolutePath
}


@Composable
private fun BuildDialogContent(projectId: String, repository: WorkspaceRepository, onOpenDiagnostic: (String, Int, Int) -> Unit) {
    val runtime = ForgeBoxRuntime
    val vm = remember(projectId) { BuildViewModel(RepoBuildBackend(repository, runtime)) }
    val linuxVm = remember { LinuxEnvViewModel(repository, runtime) }
    BuildPanel(
        projectId = projectId,
        viewModel = vm,
        onOpenDiagnostic = onOpenDiagnostic,
        onInstallPython = { linuxVm.requestPackageInstall(dev.forgebox.data.LinuxPackageManagerUi.APK, "python3 py3-pip", projectId) {} },
    )
}
@Composable
private fun LinuxEnvDialog(viewModel: LinuxEnvViewModel, projectId: String, onDismiss: () -> Unit) {
    val st by viewModel.state.collectAsState()
    LaunchedEffect(Unit) { viewModel.refresh() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Linux environment") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(st.sandboxStatement, style = MaterialTheme.typography.bodySmall)
                Text(
                    "Compatibility layer, not a security sandbox. Processes run with the app's normal Android UID; no separate Linux user isolation.",
                    style = MaterialTheme.typography.labelSmall,
                )
                val status = st.status
                if (status != null) {
                    Text("State: ${status.state}")
                    Text("Distro: ${status.distro ?: "-"} Arch: ${status.arch}")
                    Text("Rootfs present: ${status.rootfsPresent}  PRoot loader: ${status.prootLoaderPresent}")
                    Text("Shell: ${status.shellBinary}", style = MaterialTheme.typography.bodySmall)
                    if (status.rootfsPath.isNotBlank()) Text("Rootfs: ${status.rootfsPath}", style = MaterialTheme.typography.bodySmall)
                    Text("Size: ${status.sizeBytes} bytes", style = MaterialTheme.typography.bodySmall)
                    if (status.message.isNotBlank()) Text(status.message, style = MaterialTheme.typography.bodySmall)
                } else {
                    Text(if (st.loading) "Loading..." else "No status")
                }
                if (st.progressPercent > 0 || st.progressMessage.isNotBlank()) {
                    Text("Progress: ${st.progressPercent}% ${st.progressMessage}", style = MaterialTheme.typography.bodySmall)
                }
                st.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { viewModel.install(LinuxDistroUi.ALPINE, LinuxBootstrapModeUi.BUNDLED_OFFLINE) }, enabled = !st.loading) { Text("Install Alpine (bundled)") }
                    TextButton(onClick = { viewModel.install(LinuxDistroUi.ALPINE, LinuxBootstrapModeUi.NETWORK_DOWNLOAD) }, enabled = !st.loading) { Text("Download (M11)") }
                }
                Text("Bundled offline bootstrap remains default. NETWORK_DOWNLOAD path exists in proto/policy but is not runtime-verified on supported device (Option B).", style = MaterialTheme.typography.labelSmall)
                TextButton(onClick = { viewModel.uninstall() }, enabled = !st.loading && st.status?.state == LinuxEnvUiState.READY) { Text("Uninstall") }
                var pkg by remember { mutableStateOf("@mmmbuto/codex-cli-termux") }
                OutlinedTextField(value = pkg, onValueChange = { pkg = it }, label = { Text("Package spec") }, singleLine = true)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { viewModel.requestPackageInstall(dev.forgebox.data.LinuxPackageManagerUi.APK, pkg, projectId) }) { Text("apk add (PTY)") }
                }
                Text("Package install spawns a normal terminal command. Network access is governed by M11 broker/mode; guest sockets inside PRoot are not sandboxed.", style = MaterialTheme.typography.labelSmall)
                Text("No emulator runtime verification on a supported device; only API 28 emulator available (below minSdk 31).", style = MaterialTheme.typography.labelSmall)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )

}

