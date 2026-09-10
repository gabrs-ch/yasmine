package app.yasmine.ui.playlists

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.yasmine.YasmineApp
import app.yasmine.playback.PlayerConnection
import app.yasmine.ui.common.Cover
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PlaylistFfi

@Composable
fun PlaylistsScreen(player: PlayerConnection) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val revision by repo.revision.collectAsState()
    val scheme = MaterialTheme.colorScheme

    var playlists by remember { mutableStateOf<List<PlaylistFfi>>(emptyList()) }
    var open by remember { mutableStateOf<PlaylistFfi?>(null) }
    var creating by remember { mutableStateOf(false) }

    LaunchedEffect(revision) { playlists = repo.playlists() }

    val selected = open
    if (selected != null) {
        PlaylistDetail(selected, player, onBack = { open = null })
        return
    }

    Scaffold(
        containerColor = scheme.background,
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = { creating = true },
                containerColor = scheme.primary,
                contentColor = scheme.onPrimary,
                icon = { Icon(Icons.Filled.Add, null) },
                text = { Text("New") },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).padding(horizontal = 12.dp)) {
            Text(
                "Playlists",
                style = MaterialTheme.typography.titleLarge,
                color = scheme.onSurface,
                modifier = Modifier.padding(top = 12.dp, bottom = 8.dp),
            )
            if (playlists.isEmpty()) {
                Box(Modifier.fillMaxSize(), Alignment.Center) {
                    Text(
                        "No playlists yet.\nThe ones that arrive from a sync show up here.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = scheme.onSurfaceVariant,
                    )
                }
            } else {
                LazyColumn(Modifier.fillMaxSize()) {
                    items(playlists, key = { it.id }) { pl -> PlaylistRow(pl) { open = pl } }
                }
            }
        }
    }

    if (creating) {
        var name by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { creating = false },
            title = { Text("New playlist") },
            text = {
                OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") })
            },
            confirmButton = {
                TextButton(
                    enabled = name.isNotBlank(),
                    onClick = {
                        scope.launch {
                            repo.createPlaylist(name.trim())
                            playlists = repo.playlists()
                            creating = false
                        }
                    },
                ) { Text("Create") }
            },
            dismissButton = { TextButton({ creating = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun PlaylistRow(pl: PlaylistFfi, onClick: () -> Unit) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scheme = MaterialTheme.colorScheme
    var artHash by remember(pl.id) { mutableStateOf<String?>(null) }
    LaunchedEffect(pl.id) {
        val first = repo.playlistTracks(pl.id).firstOrNull()
        artHash = first?.let { repo.rows(listOf(it)).firstOrNull()?.artHash }
    }

    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Cover(artHash ?: pl.id, 96, Modifier.size(48.dp), corner = 8)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                pl.name,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = scheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text("${pl.items} tracks", fontSize = 11.5.sp, color = scheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun PlaylistDetail(pl: PlaylistFfi, player: PlayerConnection, onBack: () -> Unit) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val scheme = MaterialTheme.colorScheme
    var trackIds by remember(pl.id) { mutableStateOf<List<Long>>(emptyList()) }

    LaunchedEffect(pl.id) { trackIds = repo.playlistTracks(pl.id) }

    Column(Modifier.fillMaxSize().padding(horizontal = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 8.dp)) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back", tint = scheme.onSurface)
            }
            Text(pl.name, style = MaterialTheme.typography.titleLarge, color = scheme.onSurface)
        }
        if (trackIds.isEmpty()) {
            Box(Modifier.fillMaxSize(), Alignment.Center) {
                Text("No local tracks yet.", color = scheme.onSurfaceVariant)
            }
        } else {
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.padding(vertical = 4.dp),
            ) {
                Button(
                    onClick = { scope.launch { player.playTracks(repo.rows(trackIds), 0) } },
                    colors = androidx.compose.material3.ButtonDefaults.buttonColors(
                        containerColor = scheme.primary,
                        contentColor = scheme.onPrimary,
                    ),
                ) {
                    Icon(Icons.Filled.PlayArrow, null)
                    Spacer(Modifier.width(6.dp))
                    Text("Play all")
                }
                androidx.compose.material3.OutlinedButton(
                    onClick = { scope.launch { player.shufflePlay(repo.rows(trackIds)) } },
                ) {
                    Icon(Icons.Filled.Shuffle, null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Shuffle")
                }
            }
            LazyColumn(Modifier.fillMaxSize()) {
                items(trackIds) { id -> TrackLine(id) }
            }
        }
    }
}

@Composable
private fun TrackLine(id: Long) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scheme = MaterialTheme.colorScheme
    var label by remember(id) { mutableStateOf("…") }
    var hash by remember(id) { mutableStateOf<String?>(null) }
    LaunchedEffect(id) {
        val r = repo.rows(listOf(id)).firstOrNull()
        label = r?.let { listOfNotNull(it.artist, it.title).joinToString(" — ") } ?: "not downloaded yet"
        hash = r?.artHash
    }
    Row(
        Modifier.fillMaxWidth().padding(vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Cover(hash, 96, Modifier.size(36.dp), corner = 5)
        Spacer(Modifier.width(12.dp))
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            color = scheme.onSurface,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
