package dev.forgebox.app.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.forgebox.core.ForgeBoxRuntime
import dev.forgebox.data.WorkspaceRepository
import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun ForgeBoxApp() {
    val context = LocalContext.current
    var ready by remember { mutableStateOf(false) }
    var coreStatus by remember { mutableStateOf("Core: initializing...") }
    var selectedProjectId by rememberSaveable { mutableStateOf<String?>(null) }
    val themePref = remember { ThemePreference(context.applicationContext) }
    var themeMode by remember { mutableStateOf(themePref.getMode()) }

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
                if (response.status == ForgeProto.Status.OK && response.hasGetInfo()) "ready"
                else "Core error: ${response.error.code} ${response.error.message}"
            }.getOrElse { "Core init failed: ${it.message}" }
        }
        ready = coreStatus == "ready" || coreStatus.startsWith("Core ")
    }

    ForgeBoxTheme(mode = themeMode) {
        if (!ready) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            return@ForgeBoxTheme
        }

        val repository = remember { WorkspaceRepository(ForgeBoxRuntime, context.applicationContext.cacheDir) }
        val credentialBroker = remember {
            runCatching {
                val store = dev.forgebox.auth.SecureTokenStore(context.applicationContext)
                dev.forgebox.auth.DefaultCredentialBroker(store, dev.forgebox.auth.DeviceFlowHttpApi())
            }.getOrNull()
        }
        var openFilePath by rememberSaveable { mutableStateOf<String?>(null) }
        var openGit by rememberSaveable { mutableStateOf(false) }
        val projectId = selectedProjectId
        if (projectId == null) {
            val vm: ProjectListViewModel = viewModel(factory = ProjectListViewModel.factory(repository))
            ProjectListScreen(
                viewModel = vm,
                coreStatus = "",
                themeMode = themeMode,
                onThemeModeChange = { mode -> themeMode = mode; themePref.setMode(mode) },
                onOpenProject = { selectedProjectId = it },
            )
        } else if (openGit) {
            val vm: GitViewModel = viewModel(factory = GitViewModel.factory(repository, credentialBroker))
            GitScreen(projectId = projectId, viewModel = vm, onBack = { openGit = false })
        } else {
            val vm: ProjectExplorerViewModel = viewModel(factory = ProjectExplorerViewModel.factory(repository))
            ProjectExplorerScreen(
                projectId = projectId,
                viewModel = vm,
                onBack = { selectedProjectId = null },
                onOpenFile = { },
                onOpenFileAt = { _, _, _ -> },
                onOpenGit = { openGit = true },
            )
        }
    }
}


