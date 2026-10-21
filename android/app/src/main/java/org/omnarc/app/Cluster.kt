package org.omnarc.app

import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.HttpURLConnection
import java.net.URL
import org.json.JSONObject

data class Member(val key: String, val name: String, val platform: String, val memoryBytes: Long, val online: Boolean, val self: Boolean)
data class ClusterView(
    val name: String,
    val members: List<Member>,
    val layersHere: Int,
    val joined: Boolean = false,
    val plan: Plan? = null,
    /** Tokens this device computed today, by its own count. */
    val tokensToday: Long = 0,
)

/** The model the cluster runs, and what each device does in it. */
data class Plan(val model: String, val layers: Int, val parts: Map<String, Part>)

/** A device's layers: `[begin, end)`; a replica stands by for them. */
data class Part(val begin: Int, val end: Int, val head: Boolean, val replica: Boolean)

const val API = "http://127.0.0.1:7600"

private val PLATFORMS = listOf("Unknown", "Linux", "Windows", "macOS", "Android", "iOS", "Browser")

private fun call(method: String, contentType: String, timeoutMs: Int): HttpURLConnection =
    (URL("$API/omnarc.v1.ClusterService/$method").openConnection() as HttpURLConnection).apply {
        requestMethod = "POST"
        doOutput = true
        connectTimeout = 1500
        readTimeout = timeoutMs
        setRequestProperty("Content-Type", contentType)
        setRequestProperty("Connect-Protocol-Version", "1")
    }

/** Asks the node agent on this phone for the cluster, over the same Connect API the panel uses. Null while it starts. */
fun fetchCluster(): ClusterView? = runCatching {
    val connection = call("GetCluster", "application/proto", 3000)
    connection.outputStream.use { } // GetClusterRequest has no fields
    // the agent answers with an error while this device belongs to no cluster: it is running, just not joined
    if (connection.responseCode != HttpURLConnection.HTTP_OK) return@runCatching ClusterView("", emptyList(), 0)
    val body = connection.inputStream.use { it.readBytes() }
    var name = ""
    var self = ""
    var joined = false
    var plan: Plan? = null
    val tokens = mutableMapOf<String, Long>()
    val online = mutableSetOf<String>()
    val devices = mutableListOf<Member>()
    ProtoReader(body).forEach { field, cluster ->
        if (field != 1) return@forEach
        cluster.inner?.forEach { f, v ->
            when (f) {
                1 -> joined = true // a cluster id: alone, the agent still describes this device, without one
                2 -> name = v.string
                3 -> devices += member(v.inner!!)
                4 -> {
                    var node = ""
                    var today = 0L
                    v.inner?.forEach { t, x ->
                        if (t == 1) x.inner?.forEach { k, key -> if (k == 1) node = key.hex }
                        if (t == 10) today = x.number
                    }
                    online += node
                    tokens[node] = today
                }
                5 -> plan = plan(v.inner!!).takeIf { it.parts.isNotEmpty() } ?: plan
                6 -> v.inner?.forEach { k, key -> if (k == 1) self = key.hex }
            }
        }
    }
    val members = devices.map { it.copy(online = it.key == self || it.key in online, self = it.key == self) }
    val here = plan?.parts?.get(self)?.let { if (it.replica) 0 else it.end - it.begin } ?: 0
    ClusterView(name, members, here, joined, plan, tokens[self] ?: 0)
}.getOrNull()

// omnarc.v1.PlacementPlan: the model (2), the stages in ring order (4) and the replicas (5)
private fun plan(reader: ProtoReader): Plan {
    var model = ""
    var layers = 0
    val parts = mutableMapOf<String, Part>()
    var stage = 0
    reader.forEach { field, v ->
        when (field) {
            2 -> v.inner?.forEach { f, m -> if (f == 1) model = m.string else if (f == 4) layers = m.number.toInt() }
            4, 5 -> {
                var key = ""
                var begin = 0
                var end = 0
                v.inner?.forEach { f, x ->
                    when (f) {
                        1 -> x.inner?.forEach { k, id -> if (k == 1) key = id.hex }
                        2 -> begin = x.number.toInt()
                        3 -> end = x.number.toInt()
                    }
                }
                if (field == 4) parts[key] = Part(begin, end, head = stage++ == 0, replica = false)
                else parts.putIfAbsent(key, Part(begin, end, head = false, replica = true))
            }
        }
    }
    return Plan(model, layers, parts)
}

private fun member(device: ProtoReader): Member {
    var key = ""
    var name = ""
    var platform = 0
    var memory = 0L
    device.forEach { f, v ->
        when (f) {
            1 -> v.inner?.forEach { k, id -> if (k == 1) key = id.hex }
            2 -> name = v.string
            3 -> platform = v.number.toInt()
            7 -> memory = v.number
        }
    }
    return Member(key, name.ifEmpty { "device" }, PLATFORMS.getOrElse(platform) { "Unknown" }, memory, online = false, self = false)
}

sealed interface JoinStep {
    data object Sending : JoinStep
    data class Symbols(val symbols: List<Pair<String, String>>) : JoinStep
    data class Joined(val cluster: String) : JoinStep
    data class Failed(val message: String) : JoinStep
}

/**
 * Joins a cluster with the ticket another device shows. JoinCluster is a server stream (Connect envelopes of binary
 * messages): first the four symbols the owner compares, then the cluster's name once the owner says they match.
 */
fun joinCluster(ticket: String, name: String, onStep: (JoinStep) -> Unit) {
    runCatching {
        val connection = call("JoinCluster", "application/connect+proto", 0) // the owner may take a while to compare
        val request = ProtoWriter().string(1, ticket.trim()).string(2, name.trim()).toByteArray()
        DataOutputStream(connection.outputStream).use { it.writeByte(0); it.writeInt(request.size); it.write(request) }
        if (connection.responseCode != HttpURLConnection.HTTP_OK) {
            val error = connection.errorStream?.use { String(it.readBytes()) }.orEmpty()
            onStep(JoinStep.Failed(runCatching { JSONObject(error).optString("message") }.getOrNull().orEmpty().ifEmpty { "the agent refused the request" }))
            return
        }
        DataInputStream(connection.inputStream).use { input ->
            while (true) {
                val flags = input.readUnsignedByte()
                val payload = ByteArray(input.readInt()).also { input.readFully(it) }
                if (flags and END_STREAM != 0) {
                    // the end of the stream is JSON: empty, or the error that ended it
                    JSONObject(String(payload)).optJSONObject("error")?.let { onStep(JoinStep.Failed(it.optString("message", "the join failed"))) }
                    return
                }
                ProtoReader(payload).forEach { field, v ->
                    when (field) {
                        1 -> {
                            val symbols = mutableListOf<Pair<String, String>>()
                            v.inner?.forEach { f, symbol ->
                                if (f != 1) return@forEach
                                var emoji = ""
                                var label = ""
                                symbol.inner?.forEach { k, s -> if (k == 1) emoji = s.string else if (k == 2) label = s.string }
                                symbols += emoji to label
                            }
                            onStep(JoinStep.Symbols(symbols))
                        }
                        2 -> onStep(JoinStep.Joined(v.string))
                    }
                }
            }
        }
    }.onFailure { onStep(JoinStep.Failed(it.message ?: "the agent did not answer")) }
}

private const val END_STREAM = 0x02

/** Tells this device's agent what the owner offers (REQUIREMENTS Q1): the cluster never uses more. */
fun setBudget(memoryBytes: Long, cores: Int): Boolean = runCatching {
    val connection = call("SetBudget", "application/proto", 3000)
    val budget = ProtoWriter().uint(1, memoryBytes).uint(2, cores.toLong()).toByteArray()
    connection.outputStream.use { it.write(ProtoWriter().message(1, budget).toByteArray()) }
    connection.responseCode == HttpURLConnection.HTTP_OK
}.getOrDefault(false)

sealed interface AskStep {
    data class Piece(val text: String) : AskStep
    data class Done(val tokens: Int, val ttftMs: Float, val tpotMs: Float) : AskStep
    data class Failed(val message: String) : AskStep
}

/** Asks the model the cluster runs; the agent passes the prompt to whichever device answers. */
fun ask(prompt: String, history: List<Pair<String, String>>, onStep: (AskStep) -> Unit) {
    runCatching {
        val connection = call("Ask", "application/connect+proto", 0)
        // the conversation goes along: the model answers in its own chat template and remembers what was said
        val writer = ProtoWriter().string(1, prompt).uint(2, 256)
        for ((question, answer) in history) {
            writer.message(6, ProtoWriter().string(1, "user").string(2, question).toByteArray())
            writer.message(6, ProtoWriter().string(1, "assistant").string(2, answer).toByteArray())
        }
        writer.message(6, ProtoWriter().string(1, "user").string(2, prompt).toByteArray())
        val request = writer.toByteArray()
        DataOutputStream(connection.outputStream).use { it.writeByte(0); it.writeInt(request.size); it.write(request) }
        if (connection.responseCode != HttpURLConnection.HTTP_OK) {
            onStep(AskStep.Failed("the agent refused the question (${connection.responseCode})"))
            return
        }
        DataInputStream(connection.inputStream).use { input ->
            while (true) {
                val flags = input.readUnsignedByte()
                val payload = ByteArray(input.readInt()).also { input.readFully(it) }
                if (flags and END_STREAM != 0) {
                    JSONObject(String(payload)).optJSONObject("error")?.let { onStep(AskStep.Failed(it.optString("message", "no answer"))) }
                    return
                }
                ProtoReader(payload).forEach { field, v ->
                    when (field) {
                        1 -> onStep(AskStep.Piece(v.string))
                        2 -> {
                            var tokens = 0
                            var ttft = 0f
                            var tpot = 0f
                            v.inner?.forEach { f, x ->
                                when (f) {
                                    2 -> tokens = x.number.toInt()
                                    3 -> ttft = x.float
                                    4 -> tpot = x.float
                                }
                            }
                            onStep(AskStep.Done(tokens, ttft, tpot))
                        }
                    }
                }
            }
        }
    }.onFailure { onStep(AskStep.Failed(it.message ?: "the agent did not answer")) }
}
