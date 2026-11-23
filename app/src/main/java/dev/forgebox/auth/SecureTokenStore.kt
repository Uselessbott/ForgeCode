package dev.forgebox.auth

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

class SecureTokenStore(context: Context) : TokenStore {
    private val prefs: SharedPreferences = createPrefs(context)

    override fun put(provider: String, token: String) {
        prefs.edit().putString(key(provider), token).apply()
    }

    override fun get(provider: String): String? = prefs.getString(key(provider), null)

    override fun remove(provider: String) {
        prefs.edit().remove(key(provider)).apply()
    }

    private fun key(provider: String): String = "token_${provider.trim().lowercase()}"

    companion object {
        private fun createPrefs(context: Context): SharedPreferences {
            val masterKey = MasterKey.Builder(context)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            return EncryptedSharedPreferences.create(
                context,
                "forgebox_secure_tokens",
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
        }
    }
}
