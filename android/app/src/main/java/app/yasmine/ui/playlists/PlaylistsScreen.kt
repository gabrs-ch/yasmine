package app.yasmine.ui.playlists

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import app.yasmine.ui.common.TrackListItem
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PlaylistFfi
import uniffi.yasmine_ffi.TrackRowFfi

@Composable
fun PlaylistsScreen(player: PlayerConnection, onOpenPlayer: () -> Unit = {}) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val revision by repo.revision.collectAsState()
    val scheme = MaterialTheme.colorScheme

    var playlists by remember { mutableStateOf<List<PlaylistFfi>>(emptyList()) }
    var open by remember { mutableStateOf<PlaylistFfi?>(null) }
    var creating by remember { mutableStateOf(false) }
    var renameFor by remember { mutableStateOf<PlaylistFfi?>(null) }
    var deleteFor by remember { mutableStateOf<PlaylistFfi?>(null) }

    LaunchedEffect(revision) { playlists = repo.playlists() }

    val selected = open
    if (selected != null) {
        PlaylistDetail(selected, player, onBack = { open = null }, onOpenPlayer = onOpenPlayer)
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
                    items(playlists, key = { it.id }) { pl ->
                        PlaylistRow(
                            pl,
                            onClick = { open = pl },
                            onRename = { renameFor = pl },
                            onDelete = { deleteFor = pl },
                        )
                    }
                }
            }
        }
    }

    if (creating) {
        NameDialog("New playlist", "", confirm = "Create", onCancel = { creating = false }) { name ->
            scope.launch { repo.createPlaylist(name); playlists = repo.playlists(); creating = false }
        }
    }
    renameFor?.let { pl ->
        NameDialog("Rename playlist", pl.name, confirm = "Rename", onCancel = { renameFor = null }) { name ->
            scope.launch { repo.renamePlaylist(pl.id, name); playlists = repo.playlists(); renameFor = null }
        }
    }
    deleteFor?.let { pl ->
        AlertDialog(
            onDismissRequest = { deleteFor = null },
            title = { Text("Delete playlist?") },
            text = { Text("\"${pl.name}\" — the tracks stay in your library.", color = scheme.onSurfaceVariant) },
            confirmButton = {
                TextButton(onClick = {
                    val id = pl.id
                    deleteFor = null
                    scope.launch { repo.deletePlaylist(id); playlists = repo.playlists() }
                }) { Text("Delete", color = scheme.error) }
            },
            dismissButton = { TextButton({ deleteFor = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun NameDialog(
    title: String,
    initial: String,
    confirm: String,
    onCancel: () -> Unit,
    onConfirm: (String) -> Unit,
) {
    var name by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(title) },
        text = { OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") }) },
        confirmButton = {
            TextButton(enabled = name.isNotBlank(), onClick = { onConfirm(name.trim()) }) { Text(confirm) }
        },
        dismissButton = { TextButton(onCancel) { Text("Cancel") } },
    )
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PlaylistRow(
    pl: PlaylistFfi,
    onClick: () -> Unit,
    onRename: () -> Unit,
    onDelete: () -> Unit,
) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scheme = MaterialTheme.colorScheme
    var artHash by remember(pl.id) { mutableStateOf<String?>(null) }
    var menu by remember { mutableStateOf(false) }
    LaunchedEffect(pl.id) {
        val first = repo.playlistTracks(pl.id).firstOrNull()
        artHash = first?.let { repo.rows(listOf(it)).firstOrNull()?.artHash }
    }

    Box {
        Row(
            Modifier.fillMaxWidth()
                .combinedClickable(onClick = onClick, onLongClick = { menu = true })
                .padding(vertical = 8.dp),
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
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(text = { Text("Rename") }, onClick = { menu = false; onRename() })
            DropdownMenuItem(text = { Text("Delete") }, onClick = { menu = false; onDelete() })
        }
    }
}

@Composable
private fun PlaylistDetail(
    pl: PlaylistFfi,
    player: PlayerConnection,
    onBack: () -> Unit,
    onOpenPlayer: () -> Unit,
) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val revision by repo.revision.collectAsState()
    val scheme = MaterialTheme.colorScheme
    var rows by remember(pl.id) { mutableStateOf<List<TrackRowFfi>>(emptyList()) }

    LaunchedEffect(pl.id, revision) { rows = repo.rows(repo.playlistTracks(pl.id)) }

    Column(Modifier.fillMaxSize().padding(horizontal = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 8.dp)) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back", tint = scheme.onSurface)
            }
            Text(
                pl.name,
                style = MaterialTheme.typography.titleLarge,
                color = scheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (rows.isEmpty()) {
            Box(Modifier.fillMaxSize(), Alignment.Center) {
                Text("No local tracks yet.", color = scheme.onSurfaceVariant)
            }
        } else {
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.padding(vertical = 4.dp),
            ) {
                Button(
                    onClick = { player.playTracks(rows, 0); onOpenPlayer() },
                    colors = ButtonDefaults.buttonColors(
                        containerColor = scheme.primary,
                        contentColor = scheme.onPrimary,
                    ),
                ) {
                    Icon(Icons.Filled.PlayArrow, null)
                    Spacer(Modifier.width(6.dp))
                    Text("Play all")
                }
                OutlinedButton(onClick = { player.shufflePlay(rows); onOpenPlayer() }) {
                    Icon(Icons.Filled.Shuffle, null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Shuffle")
                }
            }
            LazyColumn(Modifier.fillMaxSize()) {
                items(rows, key = { it.id }) { row ->
                    TrackListItem(
                        title = row.title.ifBlank { "(untitled)" },
                        subtitle = listOfNotNull(row.artist, row.album).joinToString(" · ").ifBlank { "—" },
                        artHash = row.artHash,
                        coverSize = 36.dp,
                        onClick = { player.playTracks(rows, rows.indexOf(row)); onOpenPlayer() },
                        menu = { dismiss ->
                            DropdownMenuItem(
                                text = { Text("Remove from playlist") },
                                onClick = {
                                    dismiss()
                                    scope.launch { repo.playlistRemoveTrack(pl.id, row.id) }
                                },
                            )
                        },
                    )
                }
            }
        }
    }
}
