package app.yasmine.ui.pair

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import app.yasmine.YasmineApp
import app.yasmine.sync.SyncBus
import app.yasmine.sync.SyncUiState
import kotlinx.coroutines.launch
import uniffi.yasmine_ffi.PairInfoFfi
import uniffi.yasmine_ffi.SyncPhase
import uniffi.yasmine_ffi.SyncProgressFfi

/** Etapa local — o que vem antes de disparar o sync. Rodando / pronto /
 *  falhou vêm do [SyncBus] (o serviço), não daqui. */
private sealed interface Step {
    data object Scan : Step
    data class Confirm(val info: PairInfoFfi, val url: String) : Step
}

@Composable
fun PairScreen() {
    val context = LocalContext.current
    val repo = (context.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    val scheme = MaterialTheme.colorScheme
    val sync by SyncBus.state.collectAsState()

    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        )
    }
    val askCamera = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted = it }

    var step by remember { mutableStateOf<Step>(Step.Scan) }
    var manual by remember { mutableStateOf(false) }

    Column(
        Modifier.fillMaxSize().padding(16.dp).verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Sync with the PC", style = MaterialTheme.typography.titleLarge, color = scheme.onSurface)
        Text(
            "On the PC: open Yasmine and tap the phone icon (or run yasmine-sync-host). Point the camera at the code.",
            style = MaterialTheme.typography.bodyMedium,
            color = scheme.onSurfaceVariant,
        )

        when (val s = sync) {
            is SyncUiState.Running -> SyncingView(s.progress) { SyncBus.cancel(context) }

            is SyncUiState.Done -> {
                Text("Done ✓", style = MaterialTheme.typography.titleMedium, color = scheme.onSurface)
                Text(
                    "${s.tracksAdded} new tracks · ${s.playlists} playlists" +
                        if (s.mismatched > 0uL) " · ${s.mismatched} discarded" else "",
                    color = scheme.onSurfaceVariant,
                )
                PrimaryButton("Pair another", scheme) {
                    SyncBus.reset(); step = Step.Scan; manual = false
                }
            }

            is SyncUiState.Failed -> {
                Text("Failed", style = MaterialTheme.typography.titleMedium, color = scheme.onSurface)
                Text(s.message, color = scheme.error)
                PrimaryButton("Try again", scheme) {
                    SyncBus.reset(); step = Step.Scan; manual = false
                }
            }

            SyncUiState.Idle -> when (val st = step) {
                is Step.Scan -> {
                    if (!granted) {
                        PrimaryButton("Allow camera", scheme) {
                            askCamera.launch(Manifest.permission.CAMERA)
                        }
                    } else if (manual) {
                        ManualEntry(
                            scheme,
                            onCancel = { manual = false },
                            onConnect = { deviceId, addr ->
                                SyncBus.startFromAddr(context, deviceId, addr)
                            },
                        )
                    } else {
                        Box(
                            Modifier.fillMaxWidth().heightIn(max = 320.dp).aspectRatio(1f)
                                .clip(RoundedCornerShape(12.dp))
                        ) {
                            QrScanner(
                                modifier = Modifier.fillMaxSize(),
                                onQr = { raw ->
                                    if (raw.startsWith("yasmine://pair")) {
                                        scope.launch {
                                            step = runCatching { Step.Confirm(repo.parsePair(raw), raw) }
                                                .getOrElse { Step.Scan }
                                        }
                                    }
                                },
                            )
                        }
                        OutlinedButton(onClick = { manual = true }) { Text("Enter address manually") }
                    }
                }

                is Step.Confirm -> {
                    Text("Download the whole library from:", style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant)
                    Text(
                        st.info.name.ifBlank { st.info.deviceId.take(12) },
                        style = MaterialTheme.typography.titleMedium,
                        color = scheme.onSurface,
                    )
                    st.info.host?.let {
                        Text("$it:${st.info.port ?: "?"}", color = scheme.onSurfaceVariant)
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        PrimaryButton("Download", scheme) { SyncBus.startFromUrl(context, st.url) }
                        OutlinedButton(onClick = { step = Step.Scan }) { Text("Cancel") }
                    }
                }
            }
        }

        PairedList(scheme)
    }
}

@Composable
private fun PrimaryButton(label: String, scheme: androidx.compose.material3.ColorScheme, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        colors = ButtonDefaults.buttonColors(
            containerColor = scheme.primary,
            contentColor = scheme.onPrimary,
        ),
    ) { Text(label) }
}

@Composable
private fun SyncingView(p: SyncProgressFfi?, onCancel: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val phase = when (p?.phase) {
            null, SyncPhase.CONNECTING -> "Connecting…"
            SyncPhase.MERGING_USER_DATA -> "Merging playlists…"
            SyncPhase.FETCHING_LIST -> "Comparing libraries…"
            SyncPhase.DOWNLOADING -> "Downloading tracks"
            SyncPhase.INDEXING -> "Indexing…"
            SyncPhase.DONE -> "Finishing…"
        }
        Text(phase, style = MaterialTheme.typography.bodyLarge, color = scheme.onSurface)
        if (p != null && p.phase == SyncPhase.DOWNLOADING && p.tracksTotal > 0uL) {
            LinearProgressIndicator(
                progress = { p.tracksDone.toFloat() / p.tracksTotal.toFloat() },
                color = scheme.primary,
                trackColor = scheme.outline,
                modifier = Modifier.fillMaxWidth(),
            )
            val mb = { b: ULong -> "%.1f".format(b.toDouble() / 1_048_576.0) }
            Text(
                "${p.tracksDone}/${p.tracksTotal} · ${mb(p.bytesDone)}/${mb(p.bytesTotal)} MB",
                style = MaterialTheme.typography.bodySmall,
                color = scheme.onSurfaceVariant,
            )
            p.current?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, maxLines = 1) }
        } else {
            Box(Modifier.fillMaxWidth(), Alignment.Center) { CircularProgressIndicator(color = scheme.primary) }
        }
        Text(
            "You can leave this screen — it keeps going in the notification.",
            style = MaterialTheme.typography.bodySmall,
            color = scheme.onSurfaceVariant,
        )
        OutlinedButton(onClick = onCancel) { Text("Cancel") }
    }
}

@Composable
private fun ManualEntry(
    scheme: androidx.compose.material3.ColorScheme,
    onCancel: () -> Unit,
    onConnect: (deviceId: String, addr: String) -> Unit,
) {
    var deviceId by remember { mutableStateOf("") }
    var addr by remember { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(
            deviceId, { deviceId = it }, singleLine = true,
            label = { Text("device id (hex from the QR)") },
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            addr, { addr = it }, singleLine = true,
            label = { Text("ip:port") },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                enabled = deviceId.isNotBlank() && addr.contains(":"),
                onClick = { onConnect(deviceId.trim(), addr.trim()) },
                colors = ButtonDefaults.buttonColors(
                    containerColor = scheme.primary, contentColor = scheme.onPrimary,
                ),
            ) { Text("Connect") }
            OutlinedButton(onClick = onCancel) { Text("Use camera") }
        }
    }
}

@Composable
private fun PairedList(scheme: androidx.compose.material3.ColorScheme) {
    val repo = (LocalContext.current.applicationContext as YasmineApp).repo
    val scope = rememberCoroutineScope()
    var devices by remember { mutableStateOf(listOf<uniffi.yasmine_ffi.PairedDeviceFfi>()) }
    androidx.compose.runtime.LaunchedEffect(Unit) { devices = repo.pairedDevices() }
    if (devices.isEmpty()) return

    Column(Modifier.padding(top = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Paired devices", style = MaterialTheme.typography.titleSmall, color = scheme.onSurface)
        devices.forEach { d ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    d.name.ifBlank { d.deviceId.take(12) },
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = scheme.onSurface,
                )
                OutlinedButton(onClick = {
                    scope.launch { repo.unpair(d.deviceId); devices = repo.pairedDevices() }
                }) { Text("Forget") }
            }
        }
    }
}
