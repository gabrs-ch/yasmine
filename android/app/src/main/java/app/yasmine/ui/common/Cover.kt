package app.yasmine.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.yasmine.YasmineApp
import coil.compose.AsyncImage
import java.io.File

// Oito gradientes, escolhidos por hash — o mesmo truque do `.a1..a8` do
// desktop pra dar cor a quem não tem capa.
private val GRADIENTS = listOf(
    Color(0xFF6D5AE6) to Color(0xFF2A2350),
    Color(0xFFE0574B) to Color(0xFF4A1F1B),
    Color(0xFF3E9C6E) to Color(0xFF163A2A),
    Color(0xFFCF963C) to Color(0xFF3A2A16),
    Color(0xFF4E7BD8) to Color(0xFF1B2A4A),
    Color(0xFFB84FB0) to Color(0xFF3A163A),
    Color(0xFF3AA6B5) to Color(0xFF163A3F),
    Color(0xFF8C7BE6) to Color(0xFF2A2450),
)

/**
 * Capa de álbum: lê a miniatura JPEG que o Rust gerou no scan
 * (`repo.artThumbPath`) e, sem ela, cai num gradiente determinístico.
 * `size` é 96 (linha) ou 512 (destaque).
 */
@Composable
fun Cover(
    hash: String?,
    size: Int,
    modifier: Modifier = Modifier,
    corner: Int = 8,
) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val shape = RoundedCornerShape(corner.dp)
    val (c1, c2) = GRADIENTS[((hash?.hashCode() ?: 0) and 0x7fffffff) % GRADIENTS.size]
    val file = remember(hash, size) {
        repo.artThumbPath(hash, size)?.let(::File)?.takeIf { it.exists() }
    }

    Box(modifier.clip(shape).background(Brush.linearGradient(listOf(c1, c2)))) {
        if (file != null) {
            AsyncImage(
                model = file,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}
