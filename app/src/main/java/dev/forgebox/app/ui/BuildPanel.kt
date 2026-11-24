package dev.forgebox.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.forgebox.data.BuildDiagnostic
import dev.forgebox.proto.ForgeProto

@Composable
fun BuildPanel(
    projectId: String,
    viewModel: BuildViewModel,
    onOpenDiagnostic: (String, Int, Int) -> Unit,
    onInstallPython: () -> Unit,
) {
    val state by viewModel.state.collectAsState()
    var command by remember { mutableStateOf("") }
    var pyFile by remember { mutableStateOf("main.py") }
    Column(Modifier.fillMaxWidth().padding(8.dp)) {
        Text("Build / Run", style = MaterialTheme.typography.titleMedium)
        Text("Runs in Linux environment (PRoot). Not a security sandbox.", style = MaterialTheme.typography.labelSmall)
        Spacer(Modifier.height(8.dp))
        state.pythonMissingForFile?.let { f ->
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(8.dp)) {
                    Text("python3 missing in Linux environment for $f")
                    Row {
                        Button(onClick = onInstallPython) { Text("Install Python in Linux environment") }
                        Spacer(Modifier.width(8.dp))
                        TextButton(onClick = { viewModel.clearPythonMissing() }) { Text("Dismiss") }
                    }
                }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(pyFile, { pyFile = it }, label = { Text("Python file (project-relative)") }, modifier = Modifier.weight(1f))
            Button(onClick = { if (pyFile.isNotBlank()) viewModel.runPython(projectId, pyFile) }, enabled = state.runningPtyId == null) { Text("Run Python") }
        }
        Spacer(Modifier.height(4.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(command, { command = it }, label = { Text("Command in Linux session") }, modifier = Modifier.weight(1f))
            Button(onClick = { if (command.isNotBlank()) viewModel.runGeneric(projectId, command) }, enabled = state.runningPtyId == null) { Text("Run") }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (state.runningPtyId != null) Button(onClick = { viewModel.cancel() }) { Text("Cancel") }
            state.runningCommand?.let { Text("Running: $it", style = MaterialTheme.typography.labelSmall) }
        }
        Text("Resource usage: ${state.resourceSummary}", style = MaterialTheme.typography.labelSmall)
        Spacer(Modifier.height(8.dp))
        Text("Diagnostics", style = MaterialTheme.typography.titleSmall)
        LazyColumn(Modifier.height(120.dp)) {
            items(state.diagnostics) { d: BuildDiagnostic ->
                Text("${d.filePath}:${d.line}:${d.column} ${d.message}", modifier = Modifier.clickable { onOpenDiagnostic(d.filePath, d.line, d.column) }.padding(4.dp), style = MaterialTheme.typography.bodySmall)
            }
        }
        Text("Output", style = MaterialTheme.typography.titleSmall)
        Text(state.output.takeLast(4000), style = MaterialTheme.typography.bodySmall)
        Text("History", style = MaterialTheme.typography.titleSmall)
        LazyColumn(Modifier.height(120.dp)) {
            items(state.history) { h ->
                Text("${h.command} -> ${h.exitCode ?: "running"}", style = MaterialTheme.typography.bodySmall)
            }
        }
        state.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }
}
