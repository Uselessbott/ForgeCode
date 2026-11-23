package dev.forgebox.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.forgebox.data.GitFileStatusModel

@Composable
fun GitScreen(projectId: String, viewModel: GitViewModel, onBack: () -> Unit) {
    val state by viewModel.state.collectAsState()
    var commitMsg by remember { mutableStateOf("") }
    var authorName by remember { mutableStateOf("ForgeBox User") }
    var authorEmail by remember { mutableStateOf("forgebox@local") }
    var branchName by remember { mutableStateOf("") }
    var pat by remember { mutableStateOf("") }
    var showPatDialog by remember { mutableStateOf(false) }
    var showPushConfirm by remember { mutableStateOf(false) }

    LaunchedEffect(projectId) { viewModel.open(projectId) }

    Column(Modifier.fillMaxSize().padding(12.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = onBack) { Text("Back") }
            Text("Source Control", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
            TextButton(onClick = { viewModel.refresh() }) { Text("Refresh") }
        }
        Text("Project: $projectId", style = MaterialTheme.typography.labelSmall)
        Text("No emulator runtime verification on a supported device; only API 28 emulator available (below minSdk 31).", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.error)
        state.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
        state.message?.let { Text(it, color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.bodySmall) }
        if (state.loading) CircularProgressIndicator()

        LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Repository", style = MaterialTheme.typography.titleMedium)
                        Text("Branch: ${state.status?.branch ?: "(unknown)"}")
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { viewModel.initRepo() }) { Text("Init") }
                            OutlinedTextField(value = branchName, onValueChange = { branchName = it }, label = { Text("Branch name") }, modifier = Modifier.weight(1f))
                            Button(onClick = { if (branchName.isNotBlank()) viewModel.createBranch(branchName.trim()) }) { Text("Create") }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            state.branches.forEach { b ->
                                TextButton(onClick = { viewModel.switchBranch(b.name) }) { Text(if (b.isCurrent) "*${b.name}" else b.name) }
                            }
                        }
                    }
                }
            }
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Changes", style = MaterialTheme.typography.titleMedium)
                        StatusGroup("Staged", state.status?.staged.orEmpty(), onDiff = { viewModel.diff("staged", it) }, onAction = { viewModel.unstage(listOf(it)) }, actionLabel = "Unstage")
                        StatusGroup("Unstaged", state.status?.unstaged.orEmpty(), onDiff = { viewModel.diff("working", it) }, onAction = { viewModel.stage(listOf(it)) }, actionLabel = "Stage")
                        StatusGroup("Untracked", state.status?.untracked.orEmpty(), onDiff = { viewModel.diff("working", it) }, onAction = { viewModel.stage(listOf(it)) }, actionLabel = "Stage")
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { viewModel.stage((state.status?.unstaged.orEmpty() + state.status?.untracked.orEmpty()).map { it.path }) }) { Text("Stage all") }
                        }
                    }
                }
            }
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Commit", style = MaterialTheme.typography.titleMedium)
                        OutlinedTextField(value = commitMsg, onValueChange = { commitMsg = it }, label = { Text("Message") }, modifier = Modifier.fillMaxWidth())
                        OutlinedTextField(value = authorName, onValueChange = { authorName = it }, label = { Text("Author name") }, modifier = Modifier.fillMaxWidth())
                        OutlinedTextField(value = authorEmail, onValueChange = { authorEmail = it }, label = { Text("Author email") }, modifier = Modifier.fillMaxWidth())
                        Button(onClick = { viewModel.commit(commitMsg, authorName, authorEmail) }, enabled = commitMsg.isNotBlank()) { Text("Commit staged") }
                    }
                }
            }
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Diff / Show", style = MaterialTheme.typography.titleMedium)
                        Text(state.diff.ifBlank { "Select a file or commit to view diff." }, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("History", style = MaterialTheme.typography.titleMedium)
                        state.log.forEach { c ->
                            TextButton(onClick = { viewModel.showCommit(c.commitId) }) {
                                Text("${c.commitId.take(8)} ${c.message.lineSequence().firstOrNull().orEmpty()}")
                            }
                        }
                    }
                }
            }
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Auth / Remote (M9 scaffold)", style = MaterialTheme.typography.titleMedium)
                        Text("Tokens are stored via Keystore-backed encrypted storage. Raw tokens are never shown or logged.", style = MaterialTheme.typography.bodySmall)
                        Text("Device-flow and remote network operations are not runtime-verified on supported device in this environment.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                        state.deviceFlow?.let { flow ->
                            Text("User code: ${flow.userCode}")
                            Text("Verify: ${flow.verificationUri}")
                            Text("Expires in: ${flow.expiresInSeconds}s")
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { viewModel.startDeviceFlow() }) { Text("Start device flow") }
                            Button(onClick = { viewModel.pollDeviceFlow() }) { Text("Poll") }
                            Button(onClick = { showPatDialog = true }) { Text("Store PAT") }
                            Button(onClick = { viewModel.revokeAuth() }) { Text("Revoke") }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { viewModel.fetchRemote() }) { Text("Fetch") }
                            Button(onClick = { showPushConfirm = true }) { Text("Push (confirm)") }
                        }
                        state.authStatus?.let { Text("Auth status: $it", style = MaterialTheme.typography.bodySmall) }
                    }
                }
            }
        }
    }

    if (showPatDialog) {
        AlertDialog(
            onDismissRequest = { showPatDialog = false; pat = "" },
            title = { Text("Store PAT") },
            text = { OutlinedTextField(value = pat, onValueChange = { pat = it }, label = { Text("Personal Access Token") }) },
            confirmButton = { TextButton(onClick = { viewModel.storePat(pat = pat); pat = ""; showPatDialog = false }) { Text("Store") } },
            dismissButton = { TextButton(onClick = { pat = ""; showPatDialog = false }) { Text("Cancel") } }
        )
    }
    if (showPushConfirm) {
        AlertDialog(
            onDismissRequest = { showPushConfirm = false },
            title = { Text("Confirm push") },
            text = { Text("Push is a destructive-eligible remote operation. M9 requires explicit confirmation and credential handle; remote execution is not runtime-verified on supported device in this environment.") },
            confirmButton = { TextButton(onClick = { showPushConfirm = false; viewModel.pushRemote(branch = state.status?.branch ?: "main", force = false) }) { Text("Confirm push") } },
            dismissButton = { TextButton(onClick = { showPushConfirm = false }) { Text("Cancel") } }
        )
    }
}

@Composable
private fun StatusGroup(title: String, files: List<GitFileStatusModel>, onDiff: (String) -> Unit, onAction: (String) -> Unit, actionLabel: String) {
    Text("$title (${files.size})", style = MaterialTheme.typography.labelLarge)
    files.forEach { f ->
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("${f.status} ${f.path}", modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
            TextButton(onClick = { onDiff(f.path) }) { Text("Diff") }
            TextButton(onClick = { onAction(f.path) }) { Text(actionLabel) }
        }
    }
}

