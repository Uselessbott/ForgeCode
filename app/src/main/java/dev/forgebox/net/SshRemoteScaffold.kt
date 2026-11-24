package dev.forgebox.net

data class SshCommandResult(val ok: Boolean, val stdout: String, val stderr: String, val exitCode: Int, val message: String = "")

interface SshTransport {
    fun run(host: String, port: Int, username: String, credentialHandle: String, command: String): SshCommandResult
    fun cancel(sessionId: String) {}
}

class FakeSshTransport(
    private val responder: (String) -> SshCommandResult = { SshCommandResult(true, "ok\n", "", 0) },
) : SshTransport {
    val commands = mutableListOf<String>()
    val cancelled = mutableListOf<String>()
    override fun run(host: String, port: Int, username: String, credentialHandle: String, command: String): SshCommandResult {
        commands += command
        return responder(command)
    }
    override fun cancel(sessionId: String) { cancelled += sessionId }
}

class SshRemoteScaffold(private val transport: SshTransport) {
    fun runCommand(host: String, port: Int, username: String, credentialHandle: String, command: String): SshCommandResult {
        // Never log credentialHandle; caller responsible for redaction.
        return transport.run(host, port, username, credentialHandle, command)
    }
}
