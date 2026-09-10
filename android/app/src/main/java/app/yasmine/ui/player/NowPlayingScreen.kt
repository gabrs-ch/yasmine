package app.yasmine.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Repeat
import androidx.compose.material.icons.filled.RepeatOne
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.media3.common.Player
import app.yasmine.playback.PlayerConnection
import app.yasmine.ui.common.Cover

/** Tela cheia da faixa tocando — abre ao tocar numa música ou na barra. */
@Composable
fun NowPlayingScreen(player: PlayerConnection, onClose: () -> Unit) {
    val state by player.state.collectAsState()
    val scheme = MaterialTheme.colorScheme

    var scrub by remember { mutableStateOf<Float?>(null) }
    val dur = state.durationMs.coerceAtLeast(1)
    val frac = scrub ?: (state.positionMs.coerceIn(0, dur).toFloat() / dur)

    Column(
        Modifier.fillMaxSize().background(scheme.background).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onClose) {
                Icon(Icons.Filled.KeyboardArrowDown, "Close", tint = scheme.onSurface)
            }
            Spacer(Modifier.weight(1f))
            Text("Now playing", fontSize = 12.sp, color = scheme.onSurfaceVariant)
            Spacer(Modifier.weight(1f))
            Spacer(Modifier.size(48.dp))
        }

        Spacer(Modifier.height(24.dp))
        Cover(
            state.artHash, 512,
            Modifier.fillMaxWidth(0.82f).aspectRatio(1f),
            corner = 14,
        )

        Spacer(Modifier.height(28.dp))
        Text(
            state.title.ifBlank { "Nothing playing" },
            style = MaterialTheme.typography.titleLarge,
            color = scheme.onSurface,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.fillMaxWidth(),
        )
        Text(
            state.artist.ifBlank { "—" },
            fontSize = 14.sp,
            color = scheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        )

        Spacer(Modifier.height(20.dp))
        Slider(
            value = frac.coerceIn(0f, 1f),
            onValueChange = { scrub = it },
            onValueChangeFinished = { scrub?.let { player.seekTo((it * dur).toLong()) }; scrub = null },
            colors = SliderDefaults.colors(
                thumbColor = scheme.primary,
                activeTrackColor = scheme.primary,
                inactiveTrackColor = scheme.outline,
            ),
        )
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(fmt((frac * dur).toLong()), fontSize = 11.sp, color = scheme.onSurfaceVariant)
            Text(fmt(state.durationMs), fontSize = 11.sp, color = scheme.onSurfaceVariant)
        }

        Spacer(Modifier.height(12.dp))
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceEvenly,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { player.toggleShuffle() }) {
                Icon(
                    Icons.Filled.Shuffle, "Shuffle",
                    tint = if (state.shuffle) scheme.primary else scheme.onSurfaceVariant,
                )
            }
            IconButton(onClick = { player.previous() }) {
                Icon(Icons.Filled.SkipPrevious, "Previous", tint = scheme.onSurface, modifier = Modifier.size(36.dp))
            }
            Box(
                Modifier.size(64.dp).clip(CircleShape).background(scheme.primary)
                    .clickable { player.togglePlayPause() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    if (state.isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                    if (state.isPlaying) "Pause" else "Play",
                    tint = scheme.onPrimary,
                    modifier = Modifier.size(32.dp),
                )
            }
            IconButton(onClick = { player.next() }) {
                Icon(Icons.Filled.SkipNext, "Next", tint = scheme.onSurface, modifier = Modifier.size(36.dp))
            }
            IconButton(onClick = { player.cycleRepeat() }) {
                Icon(
                    if (state.repeat == Player.REPEAT_MODE_ONE) Icons.Filled.RepeatOne else Icons.Filled.Repeat,
                    "Repeat",
                    tint = if (state.repeat == Player.REPEAT_MODE_OFF) scheme.onSurfaceVariant else scheme.primary,
                )
            }
        }
    }
}

private fun fmt(ms: Long): String {
    val s = (ms.coerceAtLeast(0) / 1000)
    return "%d:%02d".format(s / 60, s % 60)
}
