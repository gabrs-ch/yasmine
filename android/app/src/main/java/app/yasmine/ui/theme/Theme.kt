package app.yasmine.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// Yasmine é escuro por natureza; o tema claro existe só pra não quebrar em
// aparelhos forçando light.
private val Dark = darkColorScheme(
    primary = Color(0xFFB9A3FF),
    onPrimary = Color(0xFF1A1030),
    secondary = Color(0xFF8FB0FF),
    background = Color(0xFF0B0B0D),
    onBackground = Color(0xFFE9E9EC),
    surface = Color(0xFF141417),
    onSurface = Color(0xFFE9E9EC),
    surfaceVariant = Color(0xFF232329),
    onSurfaceVariant = Color(0xFFB6B6BE),
    outline = Color(0xFF3A3A42),
)

private val Light = lightColorScheme(
    primary = Color(0xFF5B3FC4),
    background = Color(0xFFF6F5FA),
    surface = Color(0xFFFFFFFF),
)

@Composable
fun YasmineTheme(
    dark: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (dark) Dark else Light,
        typography = Typography(),
        content = content,
    )
}
