package app.yasmine.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.yasmine.playback.PlayerConnection
import app.yasmine.ui.common.Cover

@Composable
fun NowPlayingBar(player: PlayerConnection) {
    val state by player.state.collectAsState()
    if (!state.hasQueue) return
    val scheme = MaterialTheme.colorScheme

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp))
            .background(scheme.surface)
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Cover(state.artHash, 96, Modifier.size(44.dp), corner = 6)
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    state.title.ifBlank { "Nothing playing" },
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = scheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    state.artist.ifBlank { "—" },
                    fontSize = 11.sp,
                    color = scheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(onClick = { player.previous() }) {
                Icon(Icons.Filled.SkipPrevious, "Previous", tint = scheme.onSurfaceVariant)
            }
            Box(
                Modifier
                    .size(36.dp)
                    .clip(CircleShape)
                    .background(scheme.primary)
                    .pointerInput(Unit) { detectTapGestures { player.togglePlayPause() } },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    if (state.isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                    contentDescription = if (state.isPlaying) "Pause" else "Play",
                    tint = scheme.onPrimary,
                    modifier = Modifier.size(20.dp),
                )
            }
            IconButton(onClick = { player.next() }) {
                Icon(Icons.Filled.SkipNext, "Next", tint = scheme.onSurfaceVariant)
            }
        }

        val dur = state.durationMs.coerceAtLeast(1)
        val frac = (state.positionMs.coerceIn(0, dur).toFloat() / dur).coerceIn(0f, 1f)
        Box(
            Modifier
                .fillMaxWidth()
                .padding(top = 6.dp)
                .height(3.dp)
                .clip(RoundedCornerShape(999.dp))
                .background(scheme.outline)
                .pointerInput(dur) {
                    detectTapGestures { offset ->
                        player.seekTo((offset.x / size.width * dur).toLong())
                    }
                },
        ) {
            Box(
                Modifier
                    .fillMaxWidth(frac)
                    .height(3.dp)
                    .clip(RoundedCornerShape(999.dp))
                    .background(
                        Brush.horizontalGradient(listOf(scheme.secondary, scheme.primary))
                    ),
            )
        }
    }
}
