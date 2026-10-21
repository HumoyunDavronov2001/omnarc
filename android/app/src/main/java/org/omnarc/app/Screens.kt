package org.omnarc.app

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import omnarc.design.OmnarcTokens
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

private fun gb(bytes: Long) = "%.1f GB".format(bytes / 1_073_741_824.0)

@Composable
private fun Title(text: String, trailing: @Composable () -> Unit = {}) = Row(
    Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
) {
    Text(text, color = palette.text1, fontSize = OmnarcTokens.FontSize.heading3, fontWeight = FontWeight.SemiBold)
    trailing()
}

/** AN-02. One button's worth of screen: is this phone's power in the cluster, and on what terms right now. */
@Composable
fun ContributeScreen(cluster: ClusterView?, device: DeviceState) {
    val joined = cluster?.joined == true
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(20.dp), verticalArrangement = Arrangement.spacedBy(18.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Title("Contribute") {
            when {
                cluster == null -> Chip("Starting", dot = OmnarcTokens.Status.warning)
                joined -> Chip("Connected", active = true, dot = OmnarcTokens.Status.good)
                else -> Chip("Not in a cluster", dot = palette.text3)
            }
        }
        Ring(cluster?.members.orEmpty(), 200.dp)
        Text(if (joined) "Your power is in the cluster" else "Your power is not shared yet", color = palette.text1,
            fontSize = OmnarcTokens.FontSize.body, fontWeight = FontWeight.Medium)
        val plan = cluster?.plan
        val mine = plan?.parts?.get(cluster.members.firstOrNull { it.self }?.key)
        Text(
            when {
                !joined -> "Paste the ticket a computer's panel shows to share this phone"
                plan == null ->
                    "${cluster!!.name} · ${cluster.members.size} ${if (cluster.members.size == 1) "device" else "devices"} · no model running yet"
                mine == null -> "${plan.model} runs on the other devices: this phone's offer is too small for a layer of it"
                mine.replica -> "${plan.model} · this phone stands by for layers ${mine.begin}–${mine.end - 1}"
                else -> "${plan.model} · this phone computes layers ${mine.begin}–${mine.end - 1}"
            },
            color = palette.text2, fontSize = OmnarcTokens.FontSize.caption, textAlign = TextAlign.Center,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Chip(if (device.charging) "Charging" else "On battery ${device.batteryPercent}%", active = device.charging)
            Chip(if (device.wifi) "Wi-Fi" else "No Wi-Fi", active = device.wifi)
            device.temperatureC?.let { Chip("${it.roundToInt()} °C") }
        }
        if (cluster != null && !joined) JoinCard()
        // energy and credits are not measured yet: a dash is the honest number until they are
        Row(Modifier.fillMaxWidth()) {
            Stat(if (cluster == null) "—" else "%,d".format(cluster.tokensToday), "tokens today", modifier = Modifier.weight(1f))
            Stat("—", "Wh energy", modifier = Modifier.weight(1f))
            Stat("—", "credits", color = palette.gold, modifier = Modifier.weight(1f))
        }
    }
}

/** AN-05. The whole cluster on the phone: the Ring and every device in it. */
@Composable
fun ClusterScreen(cluster: ClusterView?) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Title(cluster?.name?.ifEmpty { "Cluster" } ?: "Cluster")
        val members = cluster?.members.orEmpty()
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
            Ring(members, 240.dp) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("${members.size}", color = palette.text1, fontSize = OmnarcTokens.FontSize.heading1, fontWeight = FontWeight.Light)
                    Text("devices", color = palette.text2, fontSize = OmnarcTokens.FontSize.caption)
                    Text("${gb(members.sumOf { it.memoryBytes })} of memory", color = palette.text3, fontSize = OmnarcTokens.FontSize.micro)
                }
            }
        }
        Card {
            if (members.isEmpty()) Text("No devices yet.", color = palette.text2, fontSize = OmnarcTokens.FontSize.base)
            for (m in members) Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(8.dp).clip(CircleShape).background(if (m.online) OmnarcTokens.Status.good else palette.text3))
                Spacer(Modifier.width(10.dp))
                Column(Modifier.weight(1f)) {
                    Text(m.name + if (m.self) " · this phone" else "", color = palette.text1, fontSize = OmnarcTokens.FontSize.base)
                    Text("${m.platform} · ${gb(m.memoryBytes)} · ${if (m.online) "online" else "offline"}", color = palette.text3, fontSize = OmnarcTokens.FontSize.small)
                }
            }
        }
    }
}

/** AN-06. Chat needs a running model; until one runs, say so instead of pretending. */
@Composable
fun ChatScreen(cluster: ClusterView?) {
    if (cluster?.plan == null) NoModel() else Conversation(cluster.plan.model)
}

/** AN-08. Asking the model the cluster runs; the answer streams in as the ring computes it. */
@Composable
private fun Conversation(model: String) {
    val scope = rememberCoroutineScope()
    val turns = remember { mutableStateListOf<Pair<String, String>>() }
    var prompt by remember { mutableStateOf("") }
    var asking by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf("") }
    Column(Modifier.fillMaxSize().imePadding().padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Title(model.ifEmpty { "Chat" })
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            for ((question, answer) in turns) {
                Text(question, color = palette.text1, fontSize = OmnarcTokens.FontSize.base,
                    modifier = Modifier.align(Alignment.End).clip(androidx.compose.foundation.shape.RoundedCornerShape(OmnarcTokens.Radius.medium))
                        .background(palette.surface2).padding(horizontal = 12.dp, vertical = 8.dp))
                Card { Text(answer.ifEmpty { "…" }, color = palette.text1, fontSize = OmnarcTokens.FontSize.base) }
            }
            if (note.isNotEmpty()) Text(note, color = palette.text3, fontSize = OmnarcTokens.FontSize.small)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(prompt, { prompt = it }, placeholder = { Text("Ask the model") }, modifier = Modifier.weight(1f))
            Button(
                onClick = {
                    val question = prompt.trim()
                    prompt = ""
                    asking = true
                    note = ""
                    turns.add(question to "")
                    val index = turns.lastIndex
                    scope.launch(Dispatchers.IO) {
                        ask(question) { step ->
                            scope.launch(Dispatchers.Main) {
                                when (step) {
                                    is AskStep.Piece -> turns[index] = question to turns[index].second + step.text
                                    is AskStep.Done -> {
                                        val speed = if (step.tpotMs > 0) " · %.1f tokens/s".format(1000 / step.tpotMs) else ""
                                        note = "${step.tokens} tokens · first after ${step.ttftMs.roundToInt()} ms$speed"
                                        asking = false
                                    }
                                    is AskStep.Failed -> { note = step.message; asking = false }
                                }
                            }
                        }
                        scope.launch(Dispatchers.Main) { asking = false }
                    }
                },
                enabled = !asking && prompt.isNotBlank(),
                colors = ButtonDefaults.buttonColors(containerColor = palette.turquoise, contentColor = palette.bg),
            ) { Text("Ask") }
        }
    }
}

@Composable
private fun NoModel() = Column(Modifier.fillMaxSize().padding(20.dp)) {
    Title("Chat")
    Spacer(Modifier.weight(1f))
    Text("No model is running in the cluster yet.", color = palette.text1, fontSize = OmnarcTokens.FontSize.body,
        modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center)
    Text("When one runs, you can ask it here — by text, or by voice on a phone too weak to hold layers.",
        color = palette.text2, fontSize = OmnarcTokens.FontSize.caption, modifier = Modifier.fillMaxWidth().padding(top = 6.dp), textAlign = TextAlign.Center)
    Spacer(Modifier.weight(1f))
}

/** AN-04 and AN-07. The owner's rules: when the phone gives power, and how much of it (Q1). */
@Composable
fun SettingsScreen(device: DeviceState) {
    val prefs = LocalContext.current.getSharedPreferences("rules", Context.MODE_PRIVATE)
    var charging by remember { mutableStateOf(prefs.getBoolean("charging", true)) }
    var wifi by remember { mutableStateOf(prefs.getBoolean("wifi", true)) }
    var screenOff by remember { mutableStateOf(prefs.getBoolean("screenOff", false)) }
    var battery by remember { mutableFloatStateOf(prefs.getFloat("battery", 40f)) }
    var temperature by remember { mutableFloatStateOf(prefs.getFloat("temperature", 40f)) }
    val maxShare = (device.memoryBytes / 1_073_741_824f).coerceAtLeast(0.5f)
    var share by remember { mutableFloatStateOf(prefs.getFloat("share", maxShare / 2)) }
    val scope = rememberCoroutineScope()
    fun save() = prefs.edit().putBoolean("charging", charging).putBoolean("wifi", wifi).putBoolean("screenOff", screenOff)
        .putFloat("battery", battery).putFloat("temperature", temperature).putFloat("share", share).apply()

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Title("When it runs")
        Card {
            Toggle("Only while charging", "Recommended", charging) { charging = it; save() }
            Toggle("Only on Wi-Fi", "No mobile data is used", wifi) { wifi = it; save() }
            Toggle("Only when the screen is off", null, screenOff) { screenOff = it; save() }
        }
        Card {
            Limit("Minimum battery", "${battery.roundToInt()}%", battery, 10f..90f) { battery = it; save() }
            Limit("Temperature limit", "${temperature.roundToInt()} °C", temperature, 30f..50f) { temperature = it; save() }
            Limit("Memory share", "%.1f GB".format(share), share, 0.25f..maxShare) {
                share = it
                save()
                // the node agent holds the cluster to it: the plan never uses more of this phone (Q1)
                val bytes = (it * 1_073_741_824L).toLong()
                scope.launch(Dispatchers.IO) { setBudget(bytes, 0) }
            }
        }
        Text("At a limit, the phone hands its work to a backup device smoothly — answers never stop.",
            color = palette.text3, fontSize = OmnarcTokens.FontSize.small)
        Text("The memory share is held by the node agent; the other rules are kept on this phone until it applies them too.",
            color = palette.text3, fontSize = OmnarcTokens.FontSize.micro)
    }
}

@Composable
private fun Toggle(label: String, note: String?, checked: Boolean, onChange: (Boolean) -> Unit) = Row(verticalAlignment = Alignment.CenterVertically) {
    Column(Modifier.weight(1f)) {
        Text(label, color = palette.text1, fontSize = OmnarcTokens.FontSize.base)
        if (note != null) Text(note, color = palette.text3, fontSize = OmnarcTokens.FontSize.small)
    }
    Switch(checked, onChange, colors = SwitchDefaults.colors(checkedTrackColor = palette.turquoise, checkedThumbColor = palette.bg))
}

@Composable
private fun Limit(label: String, value: String, current: Float, range: ClosedFloatingPointRange<Float>, onChange: (Float) -> Unit) = Column {
    Row { Text(label, color = palette.text2, fontSize = OmnarcTokens.FontSize.base, modifier = Modifier.weight(1f)); Text(value, color = palette.text1, fontWeight = FontWeight.Medium) }
    Slider(current, onChange, valueRange = range, colors = SliderDefaults.colors(thumbColor = palette.turquoise, activeTrackColor = palette.turquoise, inactiveTrackColor = palette.surface3))
}

/** AN-03. Joining with a ticket: paste it, or read the QR code with the camera app and paste what it finds. */
@Composable
private fun JoinCard() {
    val scope = rememberCoroutineScope()
    var ticket by remember { mutableStateOf("") }
    var name by remember { mutableStateOf("") }
    var step by remember { mutableStateOf<JoinStep?>(null) }
    Card {
        when (val s = step) {
            is JoinStep.Symbols -> {
                Text("Do the symbols match?", color = palette.text1, fontSize = OmnarcTokens.FontSize.heading3, fontWeight = FontWeight.SemiBold)
                Text("The computer that showed the ticket shows four symbols too. Its owner confirms there that they match.",
                    color = palette.text2, fontSize = OmnarcTokens.FontSize.caption)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
                    for ((emoji, label) in s.symbols) Column(
                        Modifier.weight(1f).clip(androidx.compose.foundation.shape.RoundedCornerShape(OmnarcTokens.Radius.medium))
                            .background(palette.surface2).padding(vertical = 10.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(emoji, fontSize = OmnarcTokens.FontSize.heading2)
                        Text(label, color = palette.text2, fontSize = OmnarcTokens.FontSize.small)
                    }
                }
            }
            is JoinStep.Joined -> Text("Joined ${s.cluster}", color = palette.text1, fontSize = OmnarcTokens.FontSize.body, fontWeight = FontWeight.Medium)
            else -> {
                Text("Join a cluster", color = palette.text1, fontSize = OmnarcTokens.FontSize.body, fontWeight = FontWeight.Medium)
                OutlinedTextField(ticket, { ticket = it }, label = { Text("The ticket the other device shows") },
                    placeholder = { Text("OMNARC:...") }, minLines = 2, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(name, { name = it }, label = { Text("This device's name (optional)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                if (s is JoinStep.Failed) Text(s.message, color = OmnarcTokens.Status.critical, fontSize = OmnarcTokens.FontSize.small)
                Button(
                    onClick = {
                        step = JoinStep.Sending
                        scope.launch(Dispatchers.IO) { joinCluster(ticket, name) { next -> scope.launch(Dispatchers.Main) { step = next } } }
                    },
                    enabled = ticket.isNotBlank() && step != JoinStep.Sending,
                    colors = ButtonDefaults.buttonColors(containerColor = palette.turquoise, contentColor = palette.bg),
                ) { Text(if (step == JoinStep.Sending) "Joining..." else "Join") }
                Text("Tip: point the camera app at the ticket's QR code and paste the text it finds here.", color = palette.text3, fontSize = OmnarcTokens.FontSize.micro)
            }
        }
    }
}
