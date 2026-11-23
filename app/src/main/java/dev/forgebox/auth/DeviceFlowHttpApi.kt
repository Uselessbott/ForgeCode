package dev.forgebox.auth

import okhttp3.FormBody
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

/**
 * Minimal GitHub device-flow client. Tokens are returned to broker only;
 * callers must never log returned token values.
 */
class DeviceFlowHttpApi(
    private val client: OkHttpClient = OkHttpClient(),
    private val clientId: String = DEFAULT_CLIENT_ID,
) : DeviceFlowApi {

    override fun startDeviceFlow(provider: String): DeviceFlowStart {
        require(provider.equals("github", ignoreCase = true)) { "Unsupported provider: $provider" }
        val body = FormBody.Builder()
            .add("client_id", clientId)
            .add("scope", "repo read:user")
            .build()
        val req = Request.Builder()
            .url("https://github.com/login/device/code")
            .header("Accept", "application/json")
            .post(body)
            .build()
        client.newCall(req).execute().use { resp ->
            val text = resp.body?.string().orEmpty()
            if (!resp.isSuccessful) error("device flow start failed: HTTP ${resp.code}")
            val json = JSONObject(text)
            return DeviceFlowStart(
                deviceCode = json.getString("device_code"),
                userCode = json.getString("user_code"),
                verificationUri = json.getString("verification_uri"),
                expiresInSeconds = json.optInt("expires_in", 900),
                intervalSeconds = json.optInt("interval", 5),
            )
        }
    }

    override fun pollDeviceFlow(provider: String, deviceCode: String): DeviceFlowPollResult {
        require(provider.equals("github", ignoreCase = true)) { "Unsupported provider: $provider" }
        val body = FormBody.Builder()
            .add("client_id", clientId)
            .add("device_code", deviceCode)
            .add("grant_type", "urn:ietf:params:oauth:grant-type:device_code")
            .build()
        val req = Request.Builder()
            .url("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .post(body)
            .build()
        client.newCall(req).execute().use { resp ->
            val text = resp.body?.string().orEmpty()
            if (!resp.isSuccessful) return DeviceFlowPollResult("error", message = "HTTP ${resp.code}")
            val json = JSONObject(text)
            val token = json.optString("access_token", "")
            if (token.isNotBlank()) return DeviceFlowPollResult(status = "authorized", accessToken = token)
            return when (json.optString("error", "unknown")) {
                "authorization_pending", "slow_down" -> DeviceFlowPollResult("pending", message = json.optString("error_description"))
                "access_denied" -> DeviceFlowPollResult("denied", message = json.optString("error_description"))
                "expired_token" -> DeviceFlowPollResult("expired", message = json.optString("error_description"))
                else -> DeviceFlowPollResult("error", message = json.optString("error_description", json.optString("error")))
            }
        }
    }

    companion object {
        // Placeholder client id for M9 wiring; production value must be configured and reviewed.
        const val DEFAULT_CLIENT_ID = "Iv1.forgebox-placeholder"
    }
}
