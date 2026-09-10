package app.yasmine.sync

import android.content.Context
import android.content.Intent
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uniffi.yasmine_ffi.SyncProgressFfi

/** O que a tela de Parear mostra sobre um sync — vivo enquanto o
 *  [SyncService] roda, independente de a tela estar montada. */
sealed interface SyncUiState {
    data object Idle : SyncUiState
    data class Running(val progress: SyncProgressFfi?) : SyncUiState
    data class Done(val tracksAdded: ULong, val playlists: UInt, val mismatched: ULong) : SyncUiState
    data class Failed(val message: String) : SyncUiState
}

/**
 * Ponte entre o [SyncService] (que segura o `pull`) e a UI. O serviço
 * publica o progresso aqui; a tela observa. Sair da tela não interrompe
 * nada — o serviço continua com a notificação.
 */
object SyncBus {
    private val _state = MutableStateFlow<SyncUiState>(SyncUiState.Idle)
    val state: StateFlow<SyncUiState> = _state.asStateFlow()

    internal fun set(s: SyncUiState) {
        _state.value = s
    }

    /** Volta pro estado neutro (depois de ver o "Pronto"/"Falhou"). */
    fun reset() {
        _state.value = SyncUiState.Idle
    }

    fun startFromUrl(ctx: Context, pairUrl: String) {
        _state.value = SyncUiState.Running(null)
        ctx.startForegroundService(
            Intent(ctx, SyncService::class.java).apply {
                action = SyncService.ACTION_START
                putExtra(SyncService.EXTRA_URL, pairUrl)
            }
        )
    }

    fun startFromAddr(ctx: Context, deviceId: String, addr: String) {
        _state.value = SyncUiState.Running(null)
        ctx.startForegroundService(
            Intent(ctx, SyncService::class.java).apply {
                action = SyncService.ACTION_START
                putExtra(SyncService.EXTRA_DEVICE_ID, deviceId)
                putExtra(SyncService.EXTRA_ADDR, addr)
            }
        )
    }

    fun cancel(ctx: Context) {
        ctx.startService(
            Intent(ctx, SyncService::class.java).apply { action = SyncService.ACTION_CANCEL }
        )
    }
}
