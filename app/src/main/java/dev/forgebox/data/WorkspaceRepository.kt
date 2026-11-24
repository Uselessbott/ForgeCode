package dev.forgebox.data

import dev.forgebox.core.ForgeBoxCore
import dev.forgebox.data.NetworkDecisionModel
import dev.forgebox.data.NetworkModeStateModel
import dev.forgebox.data.NetworkAllowlistEntryModel
import dev.forgebox.data.NetworkActivityEventModel
import dev.forgebox.data.NetworkModeUi
import dev.forgebox.data.NetworkTemporaryScopeUi
import dev.forgebox.data.NetworkActorKindUi
import dev.forgebox.proto.ForgeProto
import java.util.concurrent.atomic.AtomicInteger

class WorkspaceRepository(
    private val core: ForgeBoxCore,
    private val appCacheDir: java.io.File? = null,
) : dev.forgebox.net.NetworkRepository {
    private val ids = AtomicInteger(1)

    fun cacheDirSafe(): java.io.File = appCacheDir ?: java.io.File(System.getProperty("java.io.tmpdir"), "forgebox")

    fun listProjects(): Result<List<ProjectSummary>> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder()
                .setListProjects(ForgeProto.ListProjectsRequest.getDefaultInstance()),
        )
        response.listProjects.projectsList.map { it.toSummary() }
    }

    fun createProject(name: String): Result<ProjectSummary> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setCreateProject(
                ForgeProto.CreateProjectRequest.newBuilder().setName(name),
            ),
        )
        response.createProject.project.toSummary()
    }

    fun renameProject(projectId: String, newName: String): Result<ProjectSummary> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setRenameProject(
                ForgeProto.RenameProjectRequest.newBuilder()
                    .setProjectId(projectId)
                    .setNewName(newName),
            ),
        )
        response.renameProject.project.toSummary()
    }

    fun deleteProject(projectId: String): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setDeleteProject(
                ForgeProto.DeleteProjectRequest.newBuilder().setProjectId(projectId),
            ),
        )
    }

    fun listFiles(projectId: String, recursive: Boolean = true): Result<List<FileEntryModel>> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setListFiles(
                ForgeProto.ListFilesRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath("")
                    .setRecursive(recursive),
            ),
        )
        response.listFiles.entriesList.map {
            FileEntryModel(name = it.name, path = it.path, isDir = it.isDir, size = it.size)
        }
    }

    fun readFile(projectId: String, path: String): Result<ByteArray> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setReadFile(
                ForgeProto.ReadFileRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(path),
            ),
        )
        response.readFile.content.toByteArray()
    }

    fun statFile(projectId: String, path: String): Result<FileStatModel> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setStatFile(
                ForgeProto.StatFileRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(path),
            ),
        )
        val s = response.statFile
        FileStatModel(path = s.path, size = s.size, mtimeMillis = s.mtimeMillis, isTextHeuristic = s.isTextHeuristic)
    }

    fun getInfo(): Result<ForgeProto.GetInfoResponse> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setGetInfo(ForgeProto.GetInfoRequest.getDefaultInstance()),
        )
        response.getInfo
    }

    fun writeFile(projectId: String, path: String, content: ByteArray): Result<Long> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setWriteFile(
                ForgeProto.WriteFileRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(path)
                    .setContent(com.google.protobuf.ByteString.copyFrom(content)),
            ),
        ).writeFile.size.toLong()
    }

    fun writeFileIfUnchanged(
        projectId: String,
        path: String,
        content: ByteArray,
        expectedMtimeMillis: Long,
    ): Result<Long> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setWriteFileIfUnchanged(
                ForgeProto.WriteFileIfUnchangedRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(path)
                    .setContent(com.google.protobuf.ByteString.copyFrom(content))
                    .setExpectedMtimeMillis(expectedMtimeMillis),
            ),
        )
        response.writeFileIfUnchanged.mtimeMillis
    }

    fun createFile(projectId: String, relPath: String, content: ByteArray = ByteArray(0)): Result<Unit> =
        runCatching {
            call(
                ForgeProto.Request.newBuilder().setWriteFile(
                    ForgeProto.WriteFileRequest.newBuilder()
                        .setProjectId(projectId)
                        .setPath(relPath)
                        .setContent(com.google.protobuf.ByteString.copyFrom(content)),
                ),
            )
        }

    fun createFolder(projectId: String, relPath: String): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setCreateFolder(
                ForgeProto.CreateFolderRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(relPath),
            ),
        )
    }

    fun rename(projectId: String, oldRel: String, newRel: String): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setRenameFile(
                ForgeProto.RenameFileRequest.newBuilder()
                    .setProjectId(projectId)
                    .setOldPath(oldRel)
                    .setNewPath(newRel),
            ),
        )
    }

    fun deletePath(projectId: String, relPath: String, recursive: Boolean): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setDeletePath(
                ForgeProto.DeletePathRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(relPath)
                    .setRecursive(recursive),
            ),
        )
    }

    fun gitInit(projectId: String): Result<Unit> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitInit(ForgeProto.GitInitRequest.newBuilder().setProjectId(projectId)))
        Unit
    }

    fun gitStatus(projectId: String): Result<GitStatusModel> = runCatching {
        val r = call(ForgeProto.Request.newBuilder().setGitStatus(ForgeProto.GitStatusRequest.newBuilder().setProjectId(projectId))).gitStatus
        GitStatusModel(
            staged = r.stagedList.map { GitFileStatusModel(it.path, it.status, it.isBinary) },
            unstaged = r.unstagedList.map { GitFileStatusModel(it.path, it.status, it.isBinary) },
            untracked = r.untrackedList.map { GitFileStatusModel(it.path, it.status, it.isBinary) },
            branch = r.branch,
        )
    }

    fun gitDiff(projectId: String, scope: String, path: String = ""): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitDiff(ForgeProto.GitDiffRequest.newBuilder().setProjectId(projectId).setScope(scope).setPath(path))).gitDiff.unifiedDiff
    }

    fun gitStage(projectId: String, files: List<String>): Result<Unit> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitStage(ForgeProto.GitStageRequest.newBuilder().setProjectId(projectId).addAllFiles(files)))
        Unit
    }

    fun gitUnstage(projectId: String, files: List<String>): Result<Unit> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitUnstage(ForgeProto.GitUnstageRequest.newBuilder().setProjectId(projectId).addAllFiles(files)))
        Unit
    }

    fun gitCommit(projectId: String, message: String, authorName: String, authorEmail: String): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitCommit(ForgeProto.GitCommitRequest.newBuilder().setProjectId(projectId).setMessage(message).setAuthorName(authorName).setAuthorEmail(authorEmail))).gitCommit.commitId
    }

    fun gitBranchList(projectId: String): Result<List<GitBranchModel>> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitBranchList(ForgeProto.GitBranchListRequest.newBuilder().setProjectId(projectId))).gitBranchList.branchesList.map { GitBranchModel(it.name, it.isCurrent) }
    }

    fun gitBranchCreate(projectId: String, name: String): Result<Unit> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitBranchCreate(ForgeProto.GitBranchCreateRequest.newBuilder().setProjectId(projectId).setName(name)))
        Unit
    }

    fun gitBranchSwitch(projectId: String, name: String): Result<Unit> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitBranchSwitch(ForgeProto.GitBranchSwitchRequest.newBuilder().setProjectId(projectId).setName(name)))
        Unit
    }

    fun gitLog(projectId: String, limit: Int = 50): Result<List<GitCommitModel>> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitLog(ForgeProto.GitLogRequest.newBuilder().setProjectId(projectId).setLimit(limit))).gitLog.commitsList.map { GitCommitModel(it.commitId, it.author, it.email, it.message, it.timestamp) }
    }

    fun gitShow(projectId: String, commitId: String): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitShow(ForgeProto.GitShowRequest.newBuilder().setProjectId(projectId).setCommitId(commitId))).gitShow.unifiedDiff
    }

    fun gitCloneLocal(sourceProjectId: String, targetProjectName: String): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitCloneLocal(ForgeProto.GitCloneLocalRequest.newBuilder().setSourceProjectId(sourceProjectId).setTargetProjectName(targetProjectName))).gitCloneLocal.projectId
    }

    fun gitRemoteFetch(projectId: String, remote: String, credentialHandle: String): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitRemoteFetch(ForgeProto.GitRemoteFetchRequest.newBuilder().setProjectId(projectId).setRemote(remote).setCredentialHandle(credentialHandle))).gitRemoteFetch.message
    }

    fun gitRemotePush(projectId: String, remote: String, branch: String, force: Boolean, credentialHandle: String, approvalToken: String): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setGitRemotePush(ForgeProto.GitRemotePushRequest.newBuilder().setProjectId(projectId).setRemote(remote).setBranch(branch).setForce(force).setCredentialHandle(credentialHandle).setApprovalToken(approvalToken))).gitRemotePush.message
    }

    fun evaluateCommand(command: String, actorIsAgent: Boolean, sessionId: String, cwd: String): Result<ForgeProto.EvaluateCommandResponse> = runCatching {
        call(ForgeProto.Request.newBuilder().setEvaluateCommand(ForgeProto.EvaluateCommandRequest.newBuilder().setCommand(command).setActorIsAgent(actorIsAgent).setSessionId(sessionId).setCwd(cwd))).evaluateCommand
    }

    fun issueApprovalToken(commandHash: String, sessionId: String): Result<String> = runCatching {
        call(ForgeProto.Request.newBuilder().setIssueApprovalToken(ForgeProto.IssueApprovalTokenRequest.newBuilder().setCommandHash(commandHash).setSessionId(sessionId))).issueApprovalToken.token
    }

    fun consumeApprovalToken(token: String, commandHash: String, sessionId: String): Result<Unit> = runCatching {
        call(ForgeProto.Request.newBuilder().setConsumeApprovalToken(ForgeProto.ConsumeApprovalTokenRequest.newBuilder().setToken(token).setCommandHash(commandHash).setSessionId(sessionId)))
        Unit
    }

    fun linuxEnvStatus(): Result<LinuxEnvStatusModel> = runCatching {
        call(ForgeProto.Request.newBuilder().setLinuxEnvStatus(ForgeProto.LinuxEnvStatusRequest.getDefaultInstance())).linuxEnvStatus.toModel()
    }

    fun linuxEnvInstall(
        distro: LinuxDistroUi,
        bootstrapMode: LinuxBootstrapModeUi = LinuxBootstrapModeUi.BUNDLED_OFFLINE,
        sourcePath: String = "",
    ): Result<ForgeProto.LinuxEnvInstallResponse> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setLinuxEnvInstall(
                ForgeProto.LinuxEnvInstallRequest.newBuilder()
                    .setDistro(distro.toProto())
                    .setBootstrapMode(bootstrapMode.toProto())
                    .setSourcePath(sourcePath),
            ),
        ).linuxEnvInstall
    }

    fun linuxEnvUninstall(): Result<ForgeProto.LinuxEnvUninstallResponse> = runCatching {
        call(ForgeProto.Request.newBuilder().setLinuxEnvUninstall(ForgeProto.LinuxEnvUninstallRequest.getDefaultInstance())).linuxEnvUninstall
    }

    fun linuxShellSpawn(projectId: String, cols: Int, rows: Int, envJson: String = "{}"): Result<Long> = runCatching {
        call(ForgeProto.Request.newBuilder().setLinuxShellSpawn(ForgeProto.LinuxShellSpawnRequest.newBuilder().setProjectId(projectId).setCols(cols).setRows(rows).setEnvJson(envJson))).linuxShellSpawn.ptyId
    }

    fun linuxPackageInstall(manager: LinuxPackageManagerUi, packageSpec: String, cwdProjectId: String, requireConsent: Boolean = true): Result<ForgeProto.LinuxPackageInstallResponse> = runCatching {
        call(ForgeProto.Request.newBuilder().setLinuxPackageInstall(ForgeProto.LinuxPackageInstallRequest.newBuilder().setManager(manager.toProto()).setPackageSpec(packageSpec).setCwdProjectId(cwdProjectId).setRequireConsent(requireConsent))).linuxPackageInstall
    }

    fun linuxRunInstalledBinary(projectId: String, binaryName: String, argvJson: String, approvalToken: String, commandHash: String, sessionId: String): Result<ForgeProto.LinuxRunInstalledBinaryResponse> = runCatching {
        call(ForgeProto.Request.newBuilder().setLinuxRunInstalledBinary(ForgeProto.LinuxRunInstalledBinaryRequest.newBuilder().setProjectId(projectId).setBinaryName(binaryName).setArgvJson(argvJson).setApprovalToken(approvalToken).setCommandHash(commandHash).setSessionId(sessionId))).linuxRunInstalledBinary
    }

    override fun networkModeState(): Result<NetworkModeStateModel> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkModeState(ForgeProto.NetworkModeStateRequest.getDefaultInstance())).networkModeState.toModel()
    }

    override fun networkSetMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi, scopeId: String, durationSeconds: Int): Result<ForgeProto.NetworkSetModeResponse> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkSetMode(ForgeProto.NetworkSetModeRequest.newBuilder().setMode(mode.toProto()).setTemporaryScope(scope.toProto()).setScopeId(scopeId).setDurationSeconds(durationSeconds))).networkSetMode
    }

    override fun networkAllowlistList(): Result<List<NetworkAllowlistEntryModel>> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkAllowlistList(ForgeProto.NetworkAllowlistListRequest.getDefaultInstance())).networkAllowlistList.entriesList.map { it.toModel() }
    }

    override fun networkAllowlistUpsert(hostPattern: String, providerKind: String): Result<NetworkAllowlistEntryModel> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkAllowlistUpsert(ForgeProto.NetworkAllowlistUpsertRequest.newBuilder().setHostPattern(hostPattern).setProviderKind(providerKind))).networkAllowlistUpsert.entry.toModel()
    }

    override fun networkAllowlistRemove(hostPattern: String): Result<Boolean> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkAllowlistRemove(ForgeProto.NetworkAllowlistRemoveRequest.newBuilder().setHostPattern(hostPattern))).networkAllowlistRemove.ok
    }

    override fun networkActivityList(limit: Int): Result<List<NetworkActivityEventModel>> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkActivityList(ForgeProto.NetworkActivityListRequest.newBuilder().setLimit(limit))).networkActivityList.eventsList.map { it.toModel() }
    }

    override fun networkCancel(token: String): Result<Boolean> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkCancel(ForgeProto.NetworkCancelRequest.newBuilder().setCancelToken(token))).networkCancel.ok
    }

    override fun networkSessionPolicy(sessionId: String, enabled: Boolean): Result<ForgeProto.NetworkSessionPolicyResponse> = runCatching {
        call(ForgeProto.Request.newBuilder().setNetworkSessionPolicy(ForgeProto.NetworkSessionPolicyRequest.newBuilder().setSessionId(sessionId).setNetworkEnabled(enabled))).networkSessionPolicy
    }

    override fun networkDecision(host: String, actorKind: NetworkActorKindUi, actorId: String, sessionId: String): Result<NetworkDecisionModel> = runCatching {
        val r = call(ForgeProto.Request.newBuilder().setNetworkDecision(ForgeProto.NetworkDecisionRequest.newBuilder().setDestinationHost(host).setActorKind(actorKind.toProto()).setActorId(actorId).setSessionId(sessionId))).networkDecision
        NetworkDecisionModel(allow = r.allow, ask = r.ask, reason = r.reason, effectiveMode = r.effectiveMode.name)
    }

    fun buildRun(projectId: String, kind: ForgeProto.BuildKind, filePath: String = "", command: String = "", cols: Int = 80, rows: Int = 24): Result<BuildRunResultModel> = runCatching {
        val r = call(ForgeProto.Request.newBuilder().setBuildRun(
            ForgeProto.BuildRunRequest.newBuilder()
                .setProjectId(projectId)
                .setKind(kind)
                .setFilePath(filePath)
                .setCommand(command)
                .setCols(cols)
                .setRows(rows)
        )).buildRun
        BuildRunResultModel(r.ptyId, r.command, r.message, r.pythonMissing)
    }

    fun importProjectFromDir(name: String, dir: java.io.File): Result<ProjectSummary> = runCatching {
        val project = createProject(name).getOrThrow()
        importDirRecursive(project.id, dir, "")
        project
    }

    private fun importDirRecursive(projectId: String, dir: java.io.File, rel: String) {
        val children = dir.listFiles()?.sortedBy { it.name } ?: return
        for (child in children) {
            val childRel = if (rel.isBlank()) child.name else "$rel/${child.name}"
            if (child.isDirectory) {
                createFolder(projectId, childRel).getOrThrow()
                importDirRecursive(projectId, child, childRel)
            } else if (child.isFile) {
                createFile(projectId, childRel).getOrThrow()
                writeFile(projectId, childRel, child.readBytes()).getOrThrow()
            }
        }
    }

    private fun call(builder: ForgeProto.Request.Builder): ForgeProto.Response {
        val request = builder.setId(ids.getAndIncrement()).build()
        val response = core.call(request)
        if (response.status != ForgeProto.Status.OK) {
            val code = if (response.hasError()) response.error.code else "UNKNOWN"
            val message = if (response.hasError()) response.error.message else "unknown error"
            throw IllegalStateException("$code: $message")
        }
        return response
    }

    private fun ForgeProto.Project.toSummary() = ProjectSummary(
        id = id,
        name = name,
        path = path,
        createdAt = createdAt,
        storageBytes = storageBytes,
    )
}


