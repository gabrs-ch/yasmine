package app.yasmine

import android.app.Application
import app.yasmine.data.LibraryRepository
import app.yasmine.sync.SyncService

/**
 * Segura os singletons de longa vida: o repositório da biblioteca (que
 * embrulha a FFI Rust) é criado uma vez e usado por todas as telas e pelo
 * serviço de reprodução.
 */
class YasmineApp : Application() {

    val repo: LibraryRepository by lazy { LibraryRepository(this) }

    companion object {
        lateinit var instance: YasmineApp
            private set
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
        SyncService.ensureChannel(this)
    }
}
