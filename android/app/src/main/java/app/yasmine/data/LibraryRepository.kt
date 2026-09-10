package app.yasmine.data

import android.content.Context
import android.os.Build
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import uniffi.yasmine_ffi.YasmineLibrary
import uniffi.yasmine_ffi.PairInfoFfi
import uniffi.yasmine_ffi.PairedDeviceFfi
import uniffi.yasmine_ffi.PlaybackInfoFfi
import uniffi.yasmine_ffi.PlaylistFfi
import uniffi.yasmine_ffi.PullReportFfi
import uniffi.yasmine_ffi.SortFfi
import uniffi.yasmine_ffi.StatsFfi
import uniffi.yasmine_ffi.SyncListener
import uniffi.yasmine_ffi.SyncProgressFfi
import uniffi.yasmine_ffi.Syncer
import uniffi.yasmine_ffi.TrackRowFfi

/**
 * Única porta de entrada pra FFI. O Rust é o dono do SQLite; aqui só chamamos
 * através da fronteira, sempre em `Dispatchers.IO` (as chamadas são
 * bloqueantes). `revision` sobe depois de scan/sync pra as telas recarregarem.
 */
class LibraryRepository(context: Context) {

    /** Pasta onde o sync baixa e de onde o player lê. É a biblioteca. */
    val musicDir: File =
        (context.getExternalFilesDir("Musica") ?: File(context.filesDir, "Musica")).apply { mkdirs() }

    private val cacheDir: File = File(context.cacheDir, "yasmine").apply { mkdirs() }
    private val dbPath: String = File(context.filesDir, "library.db").absolutePath

    private val library: YasmineLibrary by lazy { YasmineLibrary.open(dbPath, cacheDir.absolutePath) }
    val syncer: Syncer by lazy { Syncer(library, deviceName()) }

    private val _revision = MutableStateFlow(0)
    val revision: StateFlow<Int> = _revision.asStateFlow()
    fun bumpRevision() { _revision.value += 1 }

    // --- biblioteca ---

    suspend fun view(sort: SortFfi): List<Long> = io { library.view(sort) }
    suspend fun search(query: String, sort: SortFfi): List<Long> = io { library.search(query, sort) }
    suspend fun rows(ids: List<Long>): List<TrackRowFfi> = io { library.rows(ids) }
    suspend fun stats(): StatsFfi = io { library.stats() }
    suspend fun playbackInfo(id: Long): PlaybackInfoFfi? = io { library.playbackInfo(id) }

    suspend fun scanMusicDir(): UInt = io {
        library.scan(musicDir.absolutePath).also { _revision.value += 1 }
    }

    // --- playlists ---

    suspend fun playlists(): List<PlaylistFfi> = io { library.playlists() }
    suspend fun playlistTracks(id: String): List<Long> = io { library.playlistTracks(id) }
    suspend fun createPlaylist(name: String): String = io { library.createPlaylist(name) }
    suspend fun renamePlaylist(id: String, name: String) = io { library.renamePlaylist(id, name) }
    suspend fun deletePlaylist(id: String) = io { library.deletePlaylist(id) }
    suspend fun playlistAppend(id: String, trackIds: List<Long>): UInt =
        io { library.playlistAppend(id, trackIds).also { _revision.value += 1 } }

    suspend fun playlistRemoveTrack(playlistId: String, trackId: Long) =
        io { library.playlistRemoveTrack(playlistId, trackId); _revision.value += 1 }

    /** Apaga o arquivo do celular + reindexa. Some até o próximo sync. */
    suspend fun deleteTrack(trackId: Long) =
        io { library.deleteTrack(trackId, musicDir.absolutePath); _revision.value += 1 }

    // --- sync ---

    suspend fun parsePair(url: String): PairInfoFfi = io { syncer.parsePair(url) }
    suspend fun pairedDevices(): List<PairedDeviceFfi> = io { syncer.pairedDevices() }
    suspend fun unpair(deviceIdHex: String) = io { syncer.unpair(deviceIdHex) }

    /** Baixa a biblioteca inteira do host do QR. Bloqueante — chama em IO. */
    suspend fun pull(pairUrl: String, onProgress: (SyncProgressFfi) -> Unit): PullReportFfi = io {
        val report = syncer.pull(
            pairUrl,
            musicDir.absolutePath,
            object : SyncListener {
                override fun onProgress(progress: SyncProgressFfi) = onProgress(progress)
            },
        )
        _revision.value += 1
        report
    }

    suspend fun pullFromAddr(
        deviceIdHex: String,
        addr: String,
        onProgress: (SyncProgressFfi) -> Unit,
    ): PullReportFfi = io {
        val report = syncer.pullFromAddr(
            deviceIdHex, addr, musicDir.absolutePath,
            object : SyncListener {
                override fun onProgress(progress: SyncProgressFfi) = onProgress(progress)
            },
        )
        _revision.value += 1
        report
    }

    fun cancelSync() = syncer.cancel()

    /** Caminho da miniatura de capa no cache do Rust (`art/<hh>/<hex>_<n>.jpg`). */
    fun artThumbPath(hash: String?, size: Int): String? {
        val h = hash ?: return null
        if (h.length < 2) return null
        return File(cacheDir, "art/${h.substring(0, 2)}/${h}_$size.jpg").absolutePath
    }

    private suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }

    private fun deviceName(): String {
        val make = Build.MANUFACTURER?.replaceFirstChar { it.uppercase() }.orEmpty()
        val model = Build.MODEL.orEmpty()
        return listOf(make, model).filter { it.isNotBlank() }.joinToString(" ").ifBlank { "Android" }
    }
}
