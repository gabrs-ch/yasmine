package app.yasmine.playback

import android.content.ComponentName
import android.content.Context
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import app.yasmine.YasmineApp
import app.yasmine.data.LibraryRepository
import com.google.common.util.concurrent.MoreExecutors
import java.io.File
import kotlin.math.pow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.yasmine_ffi.TrackRowFfi

data class PlayerState(
    val hasQueue: Boolean = false,
    val isPlaying: Boolean = false,
    val title: String = "",
    val artist: String = "",
    val artHash: String? = null,
    val positionMs: Long = 0,
    val durationMs: Long = 0,
    val trackId: Long? = null,
)

/**
 * Liga a UI ao `PlaybackService`. Monta a fila a partir das linhas que a tela
 * já tem, resolvendo o caminho de cada faixa pela FFI, e aplica o ganho do
 * nivelador (`gain_db`) como volume do ExoPlayer.
 */
class PlayerConnection(
    context: Context,
    private val repo: LibraryRepository,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val appContext = context.applicationContext

    private var controller: MediaController? = null
    private var gainByMediaId: Map<String, Float> = emptyMap()
    private var artHashByMediaId: Map<String, String?> = emptyMap()

    private val _state = MutableStateFlow(PlayerState())
    val state: StateFlow<PlayerState> = _state.asStateFlow()

    private val listener = object : Player.Listener {
        override fun onEvents(player: Player, events: Player.Events) = pushState(player)
        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            controller?.let { c ->
                mediaItem?.mediaId?.let { id -> c.volume = gainByMediaId[id] ?: 1f }
                pushState(c)
            }
        }
    }

    init {
        val token = SessionToken(appContext, ComponentName(appContext, PlaybackService::class.java))
        val future = MediaController.Builder(appContext, token).buildAsync()
        future.addListener({
            controller = future.get().also { it.addListener(listener) }
            pushState(controller)
            tickPosition()
        }, MoreExecutors.directExecutor())
    }

    fun playTracks(rows: List<TrackRowFfi>, startIndex: Int) {
        scope.launch {
            val resolved = withContext(Dispatchers.IO) {
                rows.mapNotNull { row -> repo.playbackInfo(row.id)?.let { row to it } }
            }
            if (resolved.isEmpty()) return@launch
            gainByMediaId = resolved.associate { (row, info) ->
                row.id.toString() to gainToVolume(info.gainDb)
            }
            artHashByMediaId = resolved.associate { (row, _) -> row.id.toString() to row.artHash }
            val items = resolved.map { (row, info) ->
                val artFile = repo.artThumbPath(row.artHash, 512)?.let(::File)?.takeIf { it.exists() }
                MediaItem.Builder()
                    .setMediaId(row.id.toString())
                    .setUri(Uri.fromFile(File(info.path)))
                    .setMediaMetadata(
                        MediaMetadata.Builder()
                            .setTitle(row.title)
                            .setArtist(row.artist ?: "")
                            .setAlbumTitle(row.album ?: "")
                            .apply { artFile?.let { setArtworkUri(Uri.fromFile(it)) } }
                            .build()
                    )
                    .build()
            }
            val c = controller ?: return@launch
            val start = startIndex.coerceIn(0, items.lastIndex)
            c.setMediaItems(items, start, 0L)
            c.volume = gainByMediaId[items[start].mediaId] ?: 1f
            c.prepare()
            c.play()
        }
    }

    fun togglePlayPause() = controller?.let { if (it.isPlaying) it.pause() else it.play() }
    fun next() = controller?.seekToNext()
    fun previous() = controller?.seekToPrevious()
    fun seekTo(ms: Long) = controller?.seekTo(ms)

    fun release() {
        controller?.removeListener(listener)
        controller?.release()
        controller = null
    }

    private fun tickPosition() = scope.launch {
        while (isActive) {
            controller?.let { if (it.isPlaying) pushState(it) }
            delay(500)
        }
    }

    private fun pushState(player: Player?) {
        player ?: return
        val meta = player.mediaMetadata
        val mediaId = player.currentMediaItem?.mediaId
        _state.value = PlayerState(
            hasQueue = player.mediaItemCount > 0,
            isPlaying = player.isPlaying,
            title = meta.title?.toString().orEmpty(),
            artist = meta.artist?.toString().orEmpty(),
            artHash = mediaId?.let { artHashByMediaId[it] },
            positionMs = player.currentPosition.coerceAtLeast(0),
            durationMs = player.duration.coerceAtLeast(0),
            trackId = mediaId?.toLongOrNull(),
        )
    }
}

private fun gainToVolume(gainDb: Float?): Float {
    val g = gainDb ?: return 1f
    return 10f.pow(g / 20f).coerceIn(0f, 1f)
}

/** Cria e mantém uma conexão pelo ciclo de vida da composição. */
@Composable
fun rememberPlayerConnection(): PlayerConnection {
    val context = LocalContext.current
    val repo = (context.applicationContext as YasmineApp).repo
    val connection = remember { PlayerConnection(context, repo) }
    DisposableEffect(Unit) { onDispose { connection.release() } }
    return connection
}
