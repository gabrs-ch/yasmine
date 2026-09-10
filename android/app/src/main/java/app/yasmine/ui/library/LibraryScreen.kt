package app.yasmine.ui.library

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.yasmine.YasmineApp
import app.yasmine.playback.PlayerConnection
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.SortFfi
import uniffi.yasmine_ffi.TrackRowFfi

@Composable
fun LibraryScreen(player: PlayerConnection) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val revision by repo.revision.collectAsState()

    var query by remember { mutableStateOf("") }
    var rows by remember { mutableStateOf<List<TrackRowFfi>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var scanning by remember { mutableStateOf(false) }
    var stats by remember { mutableStateOf("") }

    LaunchedEffect(query, revision) {
        loading = true
        val ids = if (query.isBlank()) repo.view(SortFfi.ARTIST_ALBUM)
        else repo.search(query, SortFfi.ARTIST_ALBUM)
        rows = repo.rows(ids)
        val s = repo.stats()
        stats = "${s.tracks} faixas · ${s.albums} álbuns · ${s.artists} artistas"
        loading = false
    }

    Column(Modifier.fillMaxSize().padding(horizontal = 12.dp)) {
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            singleLine = true,
            label = { Text("Buscar") },
            modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
        )
        Column(Modifier.fillMaxWidth().padding(bottom = 4.dp)) {
            Text(stats, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            androidx.compose.foundation.layout.Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                IconButton(
                    enabled = !scanning,
                    onClick = {
                        scanning = true
                        scope.launch {
                            runCatching { repo.scanMusicDir() }
                            scanning = false
                        }
                    },
                ) { Icon(Icons.Filled.Refresh, contentDescription = "Reindexar a pasta de música") }
                if (scanning) Text("indexando…", style = MaterialTheme.typography.labelMedium)
            }
        }

        when {
            loading -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
            rows.isEmpty() -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                Text(
                    "Biblioteca vazia.\nVá em Parear e escaneie o QR do PC.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            else -> LazyColumn(Modifier.fillMaxSize()) {
                items(rows, key = { it.id }) { row ->
                    TrackRow(row) { player.playTracks(rows, rows.indexOf(row)) }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.3f))
                }
            }
        }
    }
}

@Composable
private fun TrackRow(row: TrackRowFfi, onClick: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 10.dp),
    ) {
        Text(
            row.title.ifBlank { "(sem título)" },
            style = MaterialTheme.typography.bodyLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            listOfNotNull(row.artist, row.album).joinToString(" — ").ifBlank { "—" },
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
