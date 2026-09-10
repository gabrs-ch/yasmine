package app.yasmine.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import app.yasmine.MainActivity
import app.yasmine.R
import app.yasmine.YasmineApp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PullReportFfi
import uniffi.yasmine_ffi.SyncPhase
import uniffi.yasmine_ffi.SyncProgressFfi

/**
 * Segura o `pull` da biblioteca num foreground service: sobrevive a trocar
 * de aba, minimizar e travar a tela. Progresso vai pro [SyncBus] (a UI) e
 * pra notificação. `dataSync` no Android 15 tem orçamento de ~6h/dia — um
 * sync de biblioteca fica muito abaixo.
 */
class SyncService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var job: Job? = null
    private var lastPct = -1

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CANCEL -> {
                (application as YasmineApp).repo.cancelSync()
                job?.cancel()
                stop()
            }
            ACTION_START -> start(intent)
        }
        return START_NOT_STICKY
    }

    private fun start(intent: Intent) {
        if (job?.isActive == true) return
        lastPct = -1
        goForeground("Connecting…", null)

        val repo = (application as YasmineApp).repo
        val url = intent.getStringExtra(EXTRA_URL)
        val devId = intent.getStringExtra(EXTRA_DEVICE_ID)
        val addr = intent.getStringExtra(EXTRA_ADDR)

        job = scope.launch {
            try {
                val onProgress: (SyncProgressFfi) -> Unit = { p ->
                    SyncBus.set(SyncUiState.Running(p))
                    maybeNotify(p)
                }
                val report: PullReportFfi = when {
                    url != null -> repo.pull(url, onProgress)
                    devId != null && addr != null -> repo.pullFromAddr(devId, addr, onProgress)
                    else -> error("sync sem alvo")
                }
                SyncBus.set(
                    SyncUiState.Done(report.tracksAdded, report.playlistsMerged, report.hashMismatch)
                )
            } catch (t: Throwable) {
                SyncBus.set(SyncUiState.Failed(t.message ?: t.toString()))
            } finally {
                stop()
            }
        }
    }

    private fun stop() {
        @Suppress("DEPRECATION")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            stopForeground(true)
        }
        stopSelf()
    }

    private fun goForeground(text: String, progress: Pair<Int, Int>?) {
        val notif = buildNotification(text, progress)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NID, notif)
        }
    }

    private fun maybeNotify(p: SyncProgressFfi) {
        val pct = if (p.tracksTotal > 0uL) (p.tracksDone * 100uL / p.tracksTotal).toInt() else 0
        val key = p.phase.ordinal * 1000 + pct
        if (key == lastPct) return
        lastPct = key

        val text = when (p.phase) {
            SyncPhase.CONNECTING -> "Connecting…"
            SyncPhase.MERGING_USER_DATA -> "Merging playlists…"
            SyncPhase.FETCHING_LIST -> "Comparing libraries…"
            SyncPhase.DOWNLOADING -> "Downloading — $pct%"
            SyncPhase.INDEXING -> "Indexing…"
            SyncPhase.DONE -> "Finishing…"
        }
        val determinate = p.phase == SyncPhase.DOWNLOADING && p.tracksTotal > 0uL
        val progress = if (determinate) p.tracksDone.toInt() to p.tracksTotal.toInt() else null
        notificationManager().notify(NID, buildNotification(text, progress))
    }

    private fun buildNotification(text: String, progress: Pair<Int, Int>?): Notification {
        ensureChannel(this)
        val tap = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setContentTitle("Yasmine — syncing")
            .setContentText(text)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setOngoing(true)
            .setContentIntent(tap)
            .apply { progress?.let { (c, m) -> setProgress(m.coerceAtLeast(1), c, false) } }
            .build()
    }

    private fun notificationManager() =
        getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    override fun onDestroy() {
        job?.cancel()
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "app.yasmine.sync.START"
        const val ACTION_CANCEL = "app.yasmine.sync.CANCEL"
        const val EXTRA_URL = "url"
        const val EXTRA_DEVICE_ID = "deviceId"
        const val EXTRA_ADDR = "addr"
        private const val NID = 42
        private const val CHANNEL = "sync"

        fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val mgr = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (mgr.getNotificationChannel(CHANNEL) == null) {
                mgr.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Library sync", NotificationManager.IMPORTANCE_LOW)
                )
            }
        }
    }
}
