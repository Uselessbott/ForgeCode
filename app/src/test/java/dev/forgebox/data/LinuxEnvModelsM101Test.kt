package dev.forgebox.data

import dev.forgebox.proto.ForgeProto
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LinuxEnvModelsM101Test {
    @Test
    fun statusMapsPresenceFields() {
        val proto = ForgeProto.LinuxEnvStatusResponse.newBuilder()
            .setState(ForgeProto.LinuxEnvState.LINUX_ENV_READY)
            .setRootfsPath("/data/ForgeBox/Cache/linux/alpine/rootfs")
            .setDistro(ForgeProto.LinuxDistro.LINUX_DISTRO_ALPINE)
            .setArch("aarch64")
            .setPkgMgr("apk")
            .setMessage("Ready")
            .setRootfsPresent(true)
            .setProotLoaderPresent(true)
            .setShellBinary("/bin/sh")
            .build()
        val m = proto.toModel()
        assertEquals(LinuxEnvUiState.READY, m.state)
        assertTrue(m.rootfsPresent)
        assertTrue(m.prootLoaderPresent)
        assertEquals("/bin/sh", m.shellBinary)
    }

    @Test
    fun bootstrapModeMapsToProto() {
        assertEquals(ForgeProto.LinuxBootstrapMode.LINUX_BOOTSTRAP_MODE_BUNDLED_OFFLINE, LinuxBootstrapModeUi.BUNDLED_OFFLINE.toProto())
    }
}
