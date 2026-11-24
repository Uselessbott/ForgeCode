package dev.forgebox.app.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.forgebox.data.NetworkModeUi
import dev.forgebox.data.NetworkTemporaryScopeUi
import dev.forgebox.net.ConsentChoice

@Composable
fun NetworkScreen(vm: NetworkViewModel, sessionId: String?) {
    val st by vm.state.collectAsState()
    LaunchedEffect(Unit) { vm.refresh() }
    LaunchedEffect(st.mode?.temporaryExpiresAtMs) {
        while (st.mode?.mode == NetworkModeUi.TEMPORARY_BROADER) { vm.tickCountdown(); kotlinx.coroutines.delay(1000) }
    }
    Column(Modifier.fillMaxSize().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Network", style = MaterialTheme.typography.titleLarge)
        Text("ForgeBox does not enforce per-process network isolation. The per-session toggle kills the session's PTY on disable; the guest's own network stack may have already contacted hosts.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        Text("Linux compatibility environment. Not a security sandbox.", style = MaterialTheme.typography.bodySmall)
        st.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        st.message?.let { Text(it, color = MaterialTheme.colorScheme.primary) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(selected = st.mode?.mode == NetworkModeUi.LOCAL_ONLY, onClick = { vm.setMode(NetworkModeUi.LOCAL_ONLY) }, label = { Text("Local-only") })
            FilterChip(selected = st.mode?.mode == NetworkModeUi.CONTROLLED, onClick = { vm.setMode(NetworkModeUi.CONTROLLED) }, label = { Text("Controlled") })
            FilterChip(selected = st.mode?.mode == NetworkModeUi.TEMPORARY_BROADER, onClick = { vm.setMode(NetworkModeUi.TEMPORARY_BROADER, NetworkTemporaryScopeUi.PROJECT, sessionId ?: "project", 900) }, label = { Text("Temporary broader") })
        }
        if (st.mode?.mode == NetworkModeUi.TEMPORARY_BROADER) {
            Text("Countdown: ${st.countdownSeconds}s scope=${st.mode?.temporaryScope ?: NetworkTemporaryScopeUi.PROJECT} ${st.mode?.temporaryScopeId.orEmpty()}")
        }
        if (sessionId != null) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Session network")
                Switch(checked = st.sessionNetworkEnabled, onCheckedChange = { vm.setSessionNetworkEnabled(sessionId, it) })
            }
            st.sessionBanner?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
        Text("Allowlist", style = MaterialTheme.typography.titleMedium)
        var host by remember { mutableStateOf("") }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(value = host, onValueChange = { host = it }, label = { Text("host pattern") }, modifier = Modifier.weight(1f))
            Button(onClick = { if (host.isNotBlank()) { vm.upsertAllowlist(host.trim(), "user"); host = "" } }) { Text("Add") }
        }
        LazyColumn(Modifier.heightIn(max = 180.dp)) {
            items(st.allowlist) { e ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text("${e.hostPattern} (${e.providerKind})")
                    TextButton(onClick = { vm.removeAllowlist(e.hostPattern) }) { Text("Remove") }
                }
            }
        }
        Text("Activity", style = MaterialTheme.typography.titleMedium)
        LazyColumn(Modifier.weight(1f)) {
            items(st.activity) { a ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Column(Modifier.weight(1f)) {
                        Text("${a.destinationHost} ${a.status} ${a.actorKind}:${a.actorId}")
                        Text("up=${a.bytesUp} down=${a.bytesDown} ${a.reason}", style = MaterialTheme.typography.labelSmall)
                    }
                    if (a.status.name == "IN_FLIGHT" || a.status.name == "PENDING") TextButton(onClick = { vm.cancel(a.cancelToken) }) { Text("Cancel") }
                }
            }
        }
    }
    st.consentPrompt?.let { p ->
        AlertDialog(
            onDismissRequest = { vm.answerConsent(ConsentChoice.DENY) },
            title = { Text("Network consent") },
            text = { Text("Destination: ${p.destinationHost}\nProvider: ${p.providerKind}\nFiles: ${p.fileCount}\nBytes: ${p.byteEstimate}\nActor: ${p.actorKind}:${p.actorId}") },
            confirmButton = { TextButton(onClick = { vm.answerConsent(ConsentChoice.ALLOW_REMEMBER) }) { Text("Always allow") } },
            dismissButton = {
                Row {
                    TextButton(onClick = { vm.answerConsent(ConsentChoice.DENY) }) { Text("Deny") }
                    TextButton(onClick = { vm.answerConsent(ConsentChoice.ALLOW_ONCE) }) { Text("Allow once") }
                }
            }
        )
    }
}
