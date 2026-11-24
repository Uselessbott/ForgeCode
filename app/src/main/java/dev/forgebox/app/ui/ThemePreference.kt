package dev.forgebox.app.ui

import android.content.Context

enum class ThemeMode(val label: String) {
    SYSTEM("System"),
    DARK("Dark"),
    AMOLED("AMOLED Dark"),
}

class ThemePreference(context: Context) {
    private val prefs = context.getSharedPreferences("forgebox_ui", Context.MODE_PRIVATE)

    fun getMode(): ThemeMode {
        val raw = prefs.getString(KEY_MODE, ThemeMode.SYSTEM.name) ?: ThemeMode.SYSTEM.name
        return runCatching { ThemeMode.valueOf(raw) }.getOrDefault(ThemeMode.SYSTEM)
    }

    fun setMode(mode: ThemeMode) {
        prefs.edit().putString(KEY_MODE, mode.name).apply()
    }

    companion object {
        private const val KEY_MODE = "theme_mode"
    }
}
