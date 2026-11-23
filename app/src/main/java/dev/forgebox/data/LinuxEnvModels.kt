package dev.forgebox.data

import dev.forgebox.proto.ForgeProto

enum class LinuxEnvUiState { NOT_INSTALLED, INSTALLING, READY, FAILED }
enum class LinuxDistroUi { DEBIAN, ALPINE }
enum class LinuxPackageManagerUi { NPM, APT, APK }
enum class LinuxBootstrapModeUi { BUNDLED_OFFLINE, NETWORK_DOWNLOAD }

data class LinuxEnvStatusModel(
    val state: LinuxEnvUiState,
    val rootfsPath: String,
    val distro: LinuxDistroUi?,
    val arch: String,
    val sizeBytes: Long,
    val pkgMgr: String,
    val message: String,
    val rootfsPresent: Boolean = false,
    val prootLoaderPresent: Boolean = false,
    val shellBinary: String = "/bin/sh",
)

fun ForgeProto.LinuxEnvStatusResponse.toModel() = LinuxEnvStatusModel(
    state = when (state) {
        ForgeProto.LinuxEnvState.LINUX_ENV_INSTALLING -> LinuxEnvUiState.INSTALLING
        ForgeProto.LinuxEnvState.LINUX_ENV_READY -> LinuxEnvUiState.READY
        ForgeProto.LinuxEnvState.LINUX_ENV_FAILED -> LinuxEnvUiState.FAILED
        else -> LinuxEnvUiState.NOT_INSTALLED
    },
    rootfsPath = rootfsPath,
    distro = when (distro) {
        ForgeProto.LinuxDistro.LINUX_DISTRO_ALPINE -> LinuxDistroUi.ALPINE
        ForgeProto.LinuxDistro.LINUX_DISTRO_DEBIAN -> LinuxDistroUi.DEBIAN
        else -> null
    },
    arch = arch,
    sizeBytes = sizeBytes,
    pkgMgr = pkgMgr,
    message = message,
    rootfsPresent = rootfsPresent,
    prootLoaderPresent = prootLoaderPresent,
    shellBinary = shellBinary.ifBlank { "/bin/sh" },
)

fun LinuxBootstrapModeUi.toProto(): ForgeProto.LinuxBootstrapMode = when (this) {
    LinuxBootstrapModeUi.BUNDLED_OFFLINE -> ForgeProto.LinuxBootstrapMode.LINUX_BOOTSTRAP_MODE_BUNDLED_OFFLINE
    LinuxBootstrapModeUi.NETWORK_DOWNLOAD -> ForgeProto.LinuxBootstrapMode.LINUX_BOOTSTRAP_MODE_NETWORK_DOWNLOAD
}

fun LinuxDistroUi.toProto(): ForgeProto.LinuxDistro = when (this) {
    LinuxDistroUi.ALPINE -> ForgeProto.LinuxDistro.LINUX_DISTRO_ALPINE
    LinuxDistroUi.DEBIAN -> ForgeProto.LinuxDistro.LINUX_DISTRO_DEBIAN
}

fun LinuxPackageManagerUi.toProto(): ForgeProto.LinuxPackageManager = when (this) {
    LinuxPackageManagerUi.NPM -> ForgeProto.LinuxPackageManager.LINUX_PACKAGE_MANAGER_NPM
    LinuxPackageManagerUi.APT -> ForgeProto.LinuxPackageManager.LINUX_PACKAGE_MANAGER_APT
    LinuxPackageManagerUi.APK -> ForgeProto.LinuxPackageManager.LINUX_PACKAGE_MANAGER_APK
}
