package dev.forgebox.app.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.dp
import dev.forgebox.core.ForgeBoxRuntime
import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private enum class Destination(val label: String) {
    FILES("Files"),
    SEARCH("Search"),
    BUILD("Build"),
    SETTINGS("Settings"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ForgeBoxApp() {
    val density = LocalDensity.current
    val windowSize = LocalWindowInfo.current.containerSize
    val windowWidth = with(density) { windowSize.width.toDp() }
    val windowHeight = with(density) { windowSize.height.toDp() }
    val expanded = LayoutSpec.isExpanded(windowWidth)

    var terminalOpen by rememberSaveable { mutableStateOf(true) }
    var terminalFraction by rememberSaveable {
        mutableStateOf(LayoutSpec.DEFAULT_TERMINAL_FRACTION)
    }
    var destination by rememberSaveable { mutableStateOf(Destination.FILES) }
    var activeSession by rememberSaveable { mutableStateOf("main") }
    val sessions = remember { mutableStateListOf("main") }
    val context = LocalContext.current
    var coreStatus by remember { mutableStateOf("Core: initializing...") }

    LaunchedEffect(Unit) {
        coreStatus = withContext(Dispatchers.IO) {
            runCatching {
                ForgeBoxRuntime.ensureInitialized(context.applicationContext)
                val response = ForgeBoxRuntime.call(
                    ForgeProto.Request.newBuilder()
                        .setId(1)
                        .setGetInfo(ForgeProto.GetInfoRequest.getDefaultInstance())
                        .build(),
                )
                if (response.status == ForgeProto.Status.OK && response.hasGetInfo()) {
                    val info = response.getInfo
                    "Core ${info.version} · ${info.projectCount} projects"
                } else {
                    "Core error: ${response.error.code} ${response.error.message}"
                }
            }.getOrElse { "Core init failed: ${it.message}" }
        }
    }

    MaterialTheme {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = {
                        Column {
                            Text("ForgeBox")
                            Text(
                                text = coreStatus,
                                style = MaterialTheme.typography.labelSmall,
                            )
                        }
                    },
                    actions = {
                        IconButton(
                            onClick = {
                                sessions.clear()
                                sessions.add("main")
                                activeSession = "main"
                            },
                        ) {
                            Icon(Icons.Filled.Close, contentDescription = "Stop all sessions")
                        }
                        IconButton(onClick = { terminalOpen = !terminalOpen }) {
                            Icon(
                                imageVector = if (terminalOpen) {
                                    Icons.Filled.KeyboardArrowDown
                                } else {
                                    Icons.Filled.KeyboardArrowUp
                                },
                                contentDescription = if (terminalOpen) "Hide terminal" else "Show terminal",
                            )
                        }
                    },
                )
            },
            bottomBar = {
                if (!expanded) {
                    NavigationBar {
                        Destination.entries.forEach { item ->
                            NavigationBarItem(
                                selected = destination == item,
                                onClick = { destination = item },
                                icon = { DestinationIcon(item) },
                                label = { Text(item.label) },
                            )
                        }
                    }
                }
            },
        ) { padding ->
            Row(modifier = Modifier.fillMaxSize().padding(padding)) {
                if (expanded) {
                    NavigationRail {
                        Destination.entries.forEach { item ->
                            NavigationRailItem(
                                selected = destination == item,
                                onClick = { destination = item },
                                icon = { DestinationIcon(item) },
                                label = { Text(item.label) },
                            )
                        }
                    }
                }

                Column(modifier = Modifier.weight(1f).fillMaxHeight()) {
                    WorkspaceBar(destination = destination)

                    val editorWeight = if (terminalOpen) 1f - terminalFraction else 1f
                    Box(modifier = Modifier.weight(editorWeight).fillMaxWidth()) {
                        EditorPlaceholder()
                    }

                    if (terminalOpen) {
                        DividerHandle(
                            onDrag = { deltaPx ->
                                val deltaFraction = with(density) { deltaPx.toDp() } / windowHeight
                                terminalFraction = LayoutSpec.clampTerminalFraction(
                                    terminalFraction - deltaFraction,
                                )
                            },
                        )
                        Box(modifier = Modifier.weight(terminalFraction).fillMaxWidth()) {
                            TerminalPanel(
                                activeSession = activeSession,
                                sessions = sessions,
                                onSelect = { activeSession = it },
                                onAdd = {
                                    val name = "session-${sessions.size + 1}"
                                    sessions.add(name)
                                    activeSession = name
                                },
                                onClose = { name ->
                                    if (sessions.size > 1) {
                                        sessions.remove(name)
                                        if (activeSession == name) activeSession = sessions.last()
                                    }
                                },
                            )
                        }
                    }

                    ProgrammingToolbar()
                }
            }
        }
    }
}

@Composable
private fun DestinationIcon(destination: Destination) {
    when (destination) {
        Destination.FILES -> Icon(Icons.Filled.Home, contentDescription = null)
        Destination.SEARCH -> Icon(Icons.Filled.Search, contentDescription = null)
        Destination.BUILD -> Icon(Icons.Filled.Build, contentDescription = null)
        Destination.SETTINGS -> Icon(Icons.Filled.Settings, contentDescription = null)
    }
}

@Composable
private fun WorkspaceBar(destination: Destination) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text("Project / ${destination.label}", style = MaterialTheme.typography.titleSmall)
        Text("No project open", style = MaterialTheme.typography.labelSmall)
    }
}

@Composable
private fun EditorPlaceholder() {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text("Code editor", style = MaterialTheme.typography.headlineSmall)
            Text(
                "sora-editor host attaches in M5. M3 Rust core provides workspace/storage APIs.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun DividerHandle(onDrag: (Float) -> Unit) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(12.dp)
            .pointerInput(Unit) {
                detectDragGestures { change, dragAmount -> onDrag(dragAmount.y) }
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier = Modifier.width(48.dp).height(4.dp),
        ) {
            Box(
                modifier = Modifier.fillMaxSize(),
            ) {
                androidx.compose.material3.HorizontalDivider()
            }
        }
    }
}

@Composable
private fun TerminalPanel(
    activeSession: String,
    sessions: List<String>,
    onSelect: (String) -> Unit,
    onAdd: () -> Unit,
    onClose: (String) -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize().padding(horizontal = 12.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            sessions.forEach { name ->
                AssistChip(
                    onClick = { onSelect(name) },
                    label = { Text(name) },
                    modifier = Modifier.padding(end = 4.dp),
                )
            }
            IconButton(onClick = onAdd) {
                Text("+")
            }
        }
        Text(
            text = "cwd: ~/ForgeBox/Projects  ·  session: $activeSession",
            style = MaterialTheme.typography.labelSmall,
        )
        Box(modifier = Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Terminal", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Real PTY and shell attach in M6. Close a chip to end that session.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            Button(onClick = { onClose(activeSession) }) {
                Text("Close session")
            }
        }
    }
}

@Composable
private fun ProgrammingToolbar() {
    val keys = listOf(
        "ESC", "TAB", "CTRL", "ALT", "SHIFT", "HOME", "END", "DEL",
        "←", "↓", "↑", "→", "(", ")", "[", "]", "{", "}", "\"", "'",
        ":", ";", "=", "+", "-", "*", "/", "\\", "|", "&", "$", "#", "@", "!", "?", "<", ">",
    )
    var lastKey by remember { mutableStateOf("—") }
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(
            "Key bridge: $lastKey  (editor attaches in M5)",
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
        Row(modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
            keys.forEach { key ->
                Button(
                    onClick = { lastKey = key },
                    modifier = Modifier.padding(end = 4.dp),
                ) {
                    Text(key)
                }
            }
        }
    }
}
