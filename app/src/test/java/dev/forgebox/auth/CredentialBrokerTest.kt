package dev.forgebox.auth

import org.junit.Assert.*
import org.junit.Test

class CredentialBrokerTest {
    private class FakeStore : TokenStore {
        val map = mutableMapOf<String, String>()
        override fun put(provider: String, token: String) { map[provider] = token }
        override fun get(provider: String): String? = map[provider]
        override fun remove(provider: String) { map.remove(provider) }
    }
    private class FakeApi(var pollResult: DeviceFlowPollResult) : DeviceFlowApi {
        override fun startDeviceFlow(provider: String) = DeviceFlowStart("dc", "UC123", "https://github.com/login/device", 900, 5)
        override fun pollDeviceFlow(provider: String, deviceCode: String) = pollResult
    }

    @Test fun deviceFlowStoresTokenWithoutExposingIt() {
        val store = FakeStore()
        val api = FakeApi(DeviceFlowPollResult("authorized", accessToken = "gho_secret"))
        var now = 1000L
        val broker = DefaultCredentialBroker(store, api, nowMs = { now }, handleTtlMs = 60_000)
        val start = broker.startDeviceFlow("github")
        assertEquals("UC123", start.userCode)
        val res = broker.pollAndStore("github", "dc")
        assertEquals("authorized", res.status)
        assertNull("raw token must not be returned", res.accessToken)
        assertEquals("gho_secret", store.get("github"))
    }

    @Test fun patStoredAndHandleIssued() {
        val store = FakeStore()
        val broker = DefaultCredentialBroker(store, FakeApi(DeviceFlowPollResult("pending")), nowMs = { 1000L }, handleTtlMs = 60_000)
        val handle = broker.storePat("github", "pat_secret")
        assertTrue(handle.startsWith("cred_"))
        assertEquals("pat_secret", store.get("github"))
        val remote = broker.credentialForRemote("https://github.com/a/b.git", "fetch", "github")
        assertNotNull(remote)
        assertTrue(remote!!.handle.startsWith("cred_"))
        assertNull("handle must not contain token", remote.handle.takeIf { it.contains("pat_secret") })
    }

    @Test fun revokeClearsStoreAndHandleResolution() {
        val store = FakeStore()
        val broker = DefaultCredentialBroker(store, FakeApi(DeviceFlowPollResult("pending")), nowMs = { 1000L }, handleTtlMs = 60_000)
        broker.storePat("github", "pat_secret")
        val h = broker.credentialForRemote("https://github.com/a/b.git", "push", "github")!!
        assertEquals("pat_secret", broker.resolveHandle(h.handle))
        broker.revoke("github")
        assertNull(store.get("github"))
        assertNull(broker.resolveHandle(h.handle))
    }

    @Test fun handleExpires() {
        val store = FakeStore()
        var now = 1000L
        val broker = DefaultCredentialBroker(store, FakeApi(DeviceFlowPollResult("pending")), nowMs = { now }, handleTtlMs = 1000)
        broker.storePat("github", "pat_secret")
        val h = broker.credentialForRemote("https://github.com/a/b.git", "fetch", "github")!!
        assertEquals("pat_secret", broker.resolveHandle(h.handle))
        now += 2000
        assertNull(broker.resolveHandle(h.handle))
    }
}
