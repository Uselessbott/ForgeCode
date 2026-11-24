package dev.forgebox.net

import org.junit.Assert.*
import org.junit.Test

class SshRemoteScaffoldTest {
    @Test fun connectCommandStreamCancelAndRedactionBoundary() {
        val transport = FakeSshTransport { cmd ->
            SshCommandResult(true, "out:$cmd\n", "", 0)
        }
        val ssh = SshRemoteScaffold(transport)
        val res = ssh.runCommand("example.invalid", 22, "u", "handle-secret", "echo hi")
        assertTrue(res.ok)
        assertTrue(res.stdout.contains("echo hi"))
        assertEquals(1, transport.commands.size)
        // redaction boundary: scaffold never returns credential handle
        assertFalse(res.stdout.contains("handle-secret"))
        assertFalse(res.stderr.contains("handle-secret"))
        transport.cancel("sess-1")
        assertEquals(listOf("sess-1"), transport.cancelled)
    }
}
