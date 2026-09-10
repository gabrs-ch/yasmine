package app.yasmine.ui.common

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.yasmine.YasmineApp
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PlaylistFfi

/**
 * Linha de faixa com capa, toque (tocar) e toque longo (menu de contexto).
 * `menu` recebe uma função pra fechar o menu depois de agir.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun TrackListItem(
    title: String,
    subtitle: String,
    artHash: String?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    coverSize: Dp = 42.dp,
    menu: @Composable (dismiss: () -> Unit) -> Unit = {},
) {
    val scheme = MaterialTheme.colorScheme
    var open by remember { mutableStateOf(false) }

    Box(modifier) {
        Row(
            Modifier
                .fillMaxWidth()
                .combinedClickable(onClick = onClick, onLongClick = { open = true })
                .padding(vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Cover(artHash, 96, Modifier.size(coverSize), corner = 6)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    title,
                    style = MaterialTheme.typography.bodyLarge,
                    color = scheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    subtitle,
                    fontSize = 11.5.sp,
                    color = scheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            menu { open = false }
        }
    }
}

/** Escolhe (ou cria) uma playlist e joga [trackId] nela. Fecha via [onDone]. */
@Composable
fun AddToPlaylistDialog(trackId: Long, onDone: () -> Unit) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val scheme = MaterialTheme.colorScheme
    var playlists by remember { mutableStateOf<List<PlaylistFfi>>(emptyList()) }
    var creating by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }

    LaunchedEffect(Unit) { playlists = repo.playlists() }

    if (creating) {
        AlertDialog(
            onDismissRequest = { creating = false },
            title = { Text("New playlist") },
            text = { OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") }) },
            confirmButton = {
                TextButton(enabled = name.isNotBlank(), onClick = {
                    scope.launch {
                        val id = repo.createPlaylist(name.trim())
                        repo.playlistAppend(id, listOf(trackId))
                        onDone()
                    }
                }) { Text("Create & add") }
            },
            dismissButton = { TextButton({ creating = false }) { Text("Back") } },
        )
        return
    }

    AlertDialog(
        onDismissRequest = onDone,
        title = { Text("Add to playlist") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(
                    "+ New playlist",
                    color = scheme.primary,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.fillMaxWidth().clickable { creating = true }.padding(vertical = 12.dp),
                )
                playlists.forEach { pl ->
                    Text(
                        pl.name,
                        color = scheme.onSurface,
                        modifier = Modifier.fillMaxWidth().clickable {
                            scope.launch { repo.playlistAppend(pl.id, listOf(trackId)); onDone() }
                        }.padding(vertical = 12.dp),
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onDone) { Text("Cancel") } },
    )
}
