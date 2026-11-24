package dev.forgebox.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val LightColors = lightColorScheme()
private val DarkColors = darkColorScheme()
private val AmoledColors = darkColorScheme(
    background = Color.Black,
    surface = Color.Black,
    surfaceVariant = Color(0xFF121212),
    primary = Color(0xFF90CAF9),
    secondary = Color(0xFF80CBC4),
)

@Composable
fun ForgeBoxTheme(mode: ThemeMode, content: @Composable () -> Unit) {
    val dark = when (mode) {
        ThemeMode.SYSTEM -> isSystemInDarkTheme()
        ThemeMode.DARK, ThemeMode.AMOLED -> true
    }
    val colors = when {
        mode == ThemeMode.AMOLED -> AmoledColors
        dark -> DarkColors
        else -> LightColors
    }
    MaterialTheme(colorScheme = colors, content = content)
}
