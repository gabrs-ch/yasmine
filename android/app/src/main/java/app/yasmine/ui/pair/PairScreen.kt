package app.yasmine.ui.pair

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import app.yasmine.YasmineApp
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PairInfoFfi
import uniffi.yasmine_ffi.PullReportFfi
import uniffi.yasmine_ffi.SyncPhase
import uniffi.yasmine_ffi.SyncProgressFfi

private sealed interface PairStep {
    data object Scan : PairStep
    data class Confirm(val info: PairInfoFfi, val url: String) : PairStep
    data class Syncing(val progress: SyncProgressFfi?) : PairStep
    data class Done(val report: PullReportFfi) : PairStep
    data class Failed(val message: String) : PairStep
}

@Composable
fun PairScreen() {
    val context = LocalContext.current
    val repo = (context.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()

    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        )
    }
    val askCamera = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted = it }

    var step by remember { mutableStateOf<PairStep>(PairStep.Scan) }

    fun startSync(pull: suspend ((SyncProgressFfi) -> Unit) -> PullReportFfi) {
        step = PairStep.Syncing(null)
        scope.launch {
            step = try {
                val report = pull { p -> step = PairStep.Syncing(p) }
                PairStep.Done(report)
            } catch (t: Throwable) {
                PairStep.Failed(t.message ?: t.toString())
            }
        }
    }

    Column(
        Modifier.fillMaxSize().padding(16.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Parear com o PC", style = MaterialTheme.typography.titleLarge)
        Text(
            "No PC: rode o yasmine-sync-host. Aponte a câmera pro QR que ele mostra.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        when (val s = step) {
            is PairStep.Scan -> {
                if (!granted) {
                    Button(onClick = { askCamera.launch(Manifest.permission.CAMERA) }) {
                        Text("Permitir a câmera")
                    }
                } else {
                    QrScanner(
                        modifier = Modifier.fillMaxWidth().aspectRatio(1f),
                        onQr = { raw ->
                            if (raw.startsWith("yasmine://pair")) {
                                scope.launch {
                                    step = try {
                                        PairStep.Confirm(repo.parsePair(raw), raw)
                                    } catch (t: Throwable) {
                                        PairStep.Failed("QR inválido: ${t.message}")
                                    }
                                }
                            }
                        },
                    )
                    ManualEntry(onConnect = { deviceId, addr ->
                        startSync { onProgress -> repo.pullFromAddr(deviceId, addr, onProgress) }
                    })
                }
            }

            is PairStep.Confirm -> {
                Text("Baixar a biblioteca inteira de:", style = MaterialTheme.typography.bodyMedium)
                Text(
                    s.info.name.ifBlank { s.info.deviceId.take(12) },
                    style = MaterialTheme.typography.titleMedium,
                )
                s.info.host?.let { Text("$it:${s.info.port ?: "?"}", color = MaterialTheme.colorScheme.onSurfaceVariant) }
                androidx.compose.foundation.layout.Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { startSync { onProgress -> repo.pull(s.url, onProgress) } }) {
                        Text("Baixar")
                    }
                    OutlinedButton(onClick = { step = PairStep.Scan }) { Text("Cancelar") }
                }
            }

            is PairStep.Syncing -> SyncingView(s.progress, onCancel = {
                repo.cancelSync()
            })

            is PairStep.Done -> {
                Text("Pronto ✓", style = MaterialTheme.typography.titleMedium)
                Text(
                    "${s.report.tracksAdded} faixas novas · ${s.report.playlistsMerged} playlists" +
                        if (s.report.hashMismatch > 0uL) " · ${s.report.hashMismatch} descartadas" else "",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Button(onClick = { step = PairStep.Scan }) { Text("Parear com outro") }
            }

            is PairStep.Failed -> {
                Text("Falhou", style = MaterialTheme.typography.titleMedium)
                Text(s.message, color = MaterialTheme.colorScheme.error)
                Button(onClick = { step = PairStep.Scan }) { Text("Tentar de novo") }
            }
        }

        PairedList()
    }
}

@Composable
private fun SyncingView(p: SyncProgressFfi?, onCancel: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val phase = when (p?.phase) {
            null, SyncPhase.CONNECTING -> "conectando…"
            SyncPhase.MERGING_USER_DATA -> "juntando playlists…"
            SyncPhase.FETCHING_LIST -> "comparando bibliotecas…"
            SyncPhase.DOWNLOADING -> "baixando faixas"
            SyncPhase.INDEXING -> "indexando…"
            SyncPhase.DONE -> "finalizando…"
        }
        Text(phase, style = MaterialTheme.typography.bodyLarge)
        if (p != null && p.phase == SyncPhase.DOWNLOADING && p.tracksTotal > 0uL) {
            LinearProgressIndicator(
                progress = { p.tracksDone.toFloat() / p.tracksTotal.toFloat() },
                modifier = Modifier.fillMaxWidth(),
            )
            val mb = { b: ULong -> "%.1f".format(b.toDouble() / 1_048_576.0) }
            Text(
                "${p.tracksDone}/${p.tracksTotal} · ${mb(p.bytesDone)}/${mb(p.bytesTotal)} MB",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            p.current?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 1) }
        } else {
            Box(Modifier.fillMaxWidth(), Alignment.Center) { CircularProgressIndicator() }
        }
        OutlinedButton(onClick = onCancel) { Text("Cancelar") }
    }
}

@Composable
private fun ManualEntry(onConnect: (deviceId: String, addr: String) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    var deviceId by remember { mutableStateOf("") }
    var addr by remember { mutableStateOf("") }

    if (!expanded) {
        OutlinedButton(onClick = { expanded = true }) { Text("Digitar IP manualmente") }
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(deviceId, { deviceId = it }, singleLine = true, label = { Text("device id (hex do QR)") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(addr, { addr = it }, singleLine = true, label = { Text("ip:porta") }, modifier = Modifier.fillMaxWidth())
        Button(
            enabled = deviceId.isNotBlank() && addr.contains(":"),
            onClick = { onConnect(deviceId.trim(), addr.trim()) },
        ) { Text("Conectar") }
    }
}

@Composable
private fun PairedList() {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    var devices by remember { mutableStateOf(listOf<uniffi.yasmine_ffi.PairedDeviceFfi>()) }
    androidx.compose.runtime.LaunchedEffect(Unit) { devices = repo.pairedDevices() }
    if (devices.isEmpty()) return

    Column(Modifier.padding(top = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Dispositivos pareados", style = MaterialTheme.typography.titleSmall)
        devices.forEach { d ->
            androidx.compose.foundation.layout.Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(d.name.ifBlank { d.deviceId.take(12) }, style = MaterialTheme.typography.bodyMedium)
                OutlinedButton(onClick = {
                    scope.launch { repo.unpair(d.deviceId); devices = repo.pairedDevices() }
                }) { Text("Esquecer") }
            }
        }
    }
}
