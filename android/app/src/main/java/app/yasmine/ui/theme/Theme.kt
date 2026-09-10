package app.yasmine.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.yasmine.R

// Mesmos tokens do `:root` do app desktop (crates/pc-app/ui/src/styles/app.css).
private object Tok {
    val ground = Color(0xFF08080A)
    val card = Color(0xFF121214)
    val elevated = Color(0xFF1C1C21)
    val hover = Color(0xFF212128)
    val rule = Color(0xFF2A2A32)
    val text = Color(0xFFECECEF)
    val dim = Color(0xFFA2A2AB)
    val faint = Color(0xFF6A6A74)
    val accent = Color(0xFF7C5CFF)
    val accentBright = Color(0xFF9A82FF)
}

private val Plex = FontFamily(
    Font(R.font.ibm_plex_sans_regular, FontWeight.Normal),
    Font(R.font.ibm_plex_sans_semibold, FontWeight.SemiBold),
)
val Mono = FontFamily(Font(R.font.jetbrains_mono_regular))

private val Scheme = darkColorScheme(
    primary = Tok.accent,
    onPrimary = Color.White,
    primaryContainer = Tok.accent,
    onPrimaryContainer = Color.White,
    secondary = Tok.accentBright,
    onSecondary = Color.White,
    background = Tok.ground,
    onBackground = Tok.text,
    surface = Tok.card,
    onSurface = Tok.text,
    surfaceVariant = Tok.elevated,
    onSurfaceVariant = Tok.dim,
    surfaceContainerLowest = Tok.ground,
    surfaceContainerLow = Tok.card,
    surfaceContainer = Tok.elevated,
    surfaceContainerHigh = Tok.hover,
    surfaceContainerHighest = Tok.hover,
    outline = Tok.rule,
    outlineVariant = Tok.rule,
    error = Color(0xFFFF6B6B),
)

// Base o Typography no Plex; só ajusto o que a UI usa de fato.
private fun typography(): Typography {
    val base = Typography()
    fun TextStyle.plex(weight: FontWeight = FontWeight.Normal) =
        copy(fontFamily = Plex, fontWeight = weight)
    return base.copy(
        titleLarge = base.titleLarge.plex(FontWeight.SemiBold).copy(fontSize = 22.sp),
        titleMedium = base.titleMedium.plex(FontWeight.SemiBold),
        titleSmall = base.titleSmall.plex(FontWeight.SemiBold),
        bodyLarge = base.bodyLarge.plex().copy(fontSize = 14.sp),
        bodyMedium = base.bodyMedium.plex().copy(fontSize = 13.sp),
        bodySmall = base.bodySmall.plex().copy(fontSize = 12.sp),
        labelLarge = base.labelLarge.plex(FontWeight.SemiBold),
        labelMedium = base.labelMedium.plex().copy(fontSize = 11.sp),
        labelSmall = base.labelSmall.plex().copy(fontSize = 10.sp),
    )
}

private val YasmineShapes = Shapes(
    extraSmall = androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
    small = androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
    medium = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
    large = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
)

/** Yasmine é escuro por natureza — sem tema claro. */
@Composable
fun YasmineTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = Scheme,
        typography = typography(),
        shapes = YasmineShapes,
        content = content,
    )
}
