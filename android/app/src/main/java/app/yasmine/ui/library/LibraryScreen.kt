package app.yasmine.ui.library

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.yasmine.YasmineApp
import app.yasmine.playback.PlayerConnection
import app.yasmine.ui.common.AddToPlaylistDialog
import app.yasmine.ui.common.TrackListItem
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.SortFfi
import uniffi.yasmine_ffi.TrackRowFfi

@Composable
fun LibraryScreen(player: PlayerConnection, onOpenPlayer: () -> Unit = {}) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val revision by repo.revision.collectAsState()
    val scheme = MaterialTheme.colorScheme

    var query by remember { mutableStateOf("") }
    var rows by remember { mutableStateOf<List<TrackRowFfi>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var scanning by remember { mutableStateOf(false) }
    var stats by remember { mutableStateOf("") }
    var addTo by remember { mutableStateOf<Long?>(null) }
    var confirmDelete by remember { mutableStateOf<TrackRowFfi?>(null) }

    LaunchedEffect(query, revision) {
        loading = true
        val ids = if (query.isBlank()) repo.view(SortFfi.ARTIST_ALBUM)
        else repo.search(query, SortFfi.ARTIST_ALBUM)
        rows = repo.rows(ids)
        val s = repo.stats()
        stats = "${s.tracks} tracks · ${s.albums} albums · ${s.artists} artists"
        loading = false
    }

    Column(Modifier.fillMaxSize().padding(horizontal = 12.dp)) {
        Text(
            "Your Library",
            style = MaterialTheme.typography.titleLarge,
            color = scheme.onSurface,
            modifier = Modifier.padding(top = 12.dp, bottom = 8.dp),
        )

        SearchPill(query, { query = it }, scheme)

        Row(
            Modifier.fillMaxWidth().padding(vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Button(
                enabled = rows.isNotEmpty(),
                onClick = { player.shufflePlay(rows) },
                colors = ButtonDefaults.buttonColors(
                    containerColor = scheme.primary,
                    contentColor = scheme.onPrimary,
                ),
                contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp),
            ) {
                Icon(Icons.Filled.Shuffle, null, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text("Shuffle", fontSize = 13.sp)
            }
            Spacer(Modifier.width(10.dp))
            Text(stats, fontSize = 11.sp, color = scheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            if (scanning) {
                Text("scanning…", fontSize = 11.sp, color = scheme.onSurfaceVariant)
                Spacer(Modifier.width(6.dp))
            }
            IconButton(
                enabled = !scanning,
                onClick = {
                    scanning = true
                    scope.launch {
                        runCatching { repo.scanMusicDir() }
                        scanning = false
                    }
                },
                modifier = Modifier.size(32.dp),
            ) { Icon(Icons.Filled.Refresh, "Rescan folder", tint = scheme.onSurfaceVariant) }
        }

        when {
            loading -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator(color = scheme.primary) }
            rows.isEmpty() -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                Text(
                    "Your library is empty.\nGo to Sync and scan the PC's QR code.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = scheme.onSurfaceVariant,
                )
            }
            else -> LazyColumn(Modifier.fillMaxSize()) {
                items(rows, key = { it.id }) { row ->
                    TrackListItem(
                        title = row.title.ifBlank { "(untitled)" },
                        subtitle = listOfNotNull(row.artist, row.album).joinToString(" · ").ifBlank { "—" },
                        artHash = row.artHash,
                        onClick = {
                            player.playTracks(rows, rows.indexOf(row))
                            onOpenPlayer()
                        },
                        menu = { dismiss ->
                            DropdownMenuItem(
                                text = { Text("Add to playlist") },
                                onClick = { dismiss(); addTo = row.id },
                            )
                            DropdownMenuItem(
                                text = { Text("Delete from library") },
                                onClick = { dismiss(); confirmDelete = row },
                            )
                        },
                    )
                }
            }
        }
    }

    addTo?.let { id ->
        AddToPlaylistDialog(trackId = id, onDone = { addTo = null })
    }

    confirmDelete?.let { row ->
        AlertDialog(
            onDismissRequest = { confirmDelete = null },
            title = { Text("Delete from library?") },
            text = {
                Text(
                    "\"${row.title}\" — removes the file from this phone and " +
                        "re-scans. It comes back on the next sync.",
                    color = scheme.onSurfaceVariant,
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    val t = row
                    confirmDelete = null
                    scope.launch { runCatching { repo.deleteTrack(t.id) } }
                }) { Text("Delete", color = scheme.error) }
            },
            dismissButton = { TextButton({ confirmDelete = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun SearchPill(
    value: String,
    onChange: (String) -> Unit,
    scheme: androidx.compose.material3.ColorScheme,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(38.dp)
            .background(scheme.surfaceVariant, RoundedCornerShape(999.dp))
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Filled.Search, null, tint = scheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
        Spacer(Modifier.width(8.dp))
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (value.isEmpty()) {
                Text("Search your library", fontSize = 13.sp, color = scheme.onSurfaceVariant)
            }
            BasicTextField(
                value = value,
                onValueChange = onChange,
                singleLine = true,
                textStyle = TextStyle(color = scheme.onSurface, fontSize = 13.sp),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(scheme.primary),
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

