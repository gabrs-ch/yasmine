package app.yasmine.ui.playlists

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.unit.dp
import app.yasmine.YasmineApp
import app.yasmine.playback.PlayerConnection
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PlaylistFfi
import uniffi.yasmine_ffi.SortFfi

@Composable
fun PlaylistsScreen(player: PlayerConnection) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val revision by repo.revision.collectAsState()

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
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = { creating = true },
                icon = { Icon(Icons.Filled.Add, null) },
                text = { Text("Nova") },
            )
        },
    ) { padding ->
        if (playlists.isEmpty()) {
            Box(Modifier.fillMaxSize().padding(padding), Alignment.Center) {
                Text(
                    "Sem playlists.\nAs que vierem no sync aparecem aqui.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        } else {
            LazyColumn(Modifier.fillMaxSize().padding(padding)) {
                items(playlists, key = { it.id }) { pl ->
                    Column(
                        Modifier.fillMaxWidth().clickable { open = pl }.padding(16.dp),
                    ) {
                        Text(pl.name, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "${pl.items} faixas",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.3f))
                }
            }
        }
    }

    if (creating) {
        var name by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { creating = false },
            title = { Text("Nova playlist") },
            text = {
                OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Nome") })
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
                ) { Text("Criar") }
            },
            dismissButton = { TextButton({ creating = false }) { Text("Cancelar") } },
        )
    }
}

@Composable
private fun PlaylistDetail(pl: PlaylistFfi, player: PlayerConnection, onBack: () -> Unit) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    var trackIds by remember(pl.id) { mutableStateOf<List<Long>>(emptyList()) }

    LaunchedEffect(pl.id) { trackIds = repo.playlistTracks(pl.id) }

    Column(Modifier.fillMaxSize()) {
        androidx.compose.foundation.layout.Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Voltar")
            }
            Text(pl.name, style = MaterialTheme.typography.titleLarge)
        }
        if (trackIds.isEmpty()) {
            Box(Modifier.fillMaxSize(), Alignment.Center) {
                Text("Sem faixas locais ainda.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        } else {
            TextButton(onClick = {
                scope.launch { player.playTracks(repo.rows(trackIds), 0) }
            }) {
                Icon(Icons.Filled.PlayArrow, null)
                Text("Tocar tudo")
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
    var label by remember(id) { mutableStateOf("…") }
    LaunchedEffect(id) {
        val r = repo.rows(listOf(id)).firstOrNull()
        label = r?.let { listOfNotNull(it.artist, it.title).joinToString(" — ") } ?: "faixa ainda não baixada"
    }
    Text(label, Modifier.fillMaxWidth().padding(16.dp), style = MaterialTheme.typography.bodyMedium)
}
