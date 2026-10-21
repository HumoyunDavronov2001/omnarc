package org.omnarc.app

import java.io.ByteArrayOutputStream

/**
 * Just enough Protocol Buffers for the agent's API, which speaks binary messages only (the same as the panel).
 * A reader walks one message's fields; a writer builds a request of strings.
 */
class ProtoReader(private val bytes: ByteArray, private var at: Int = 0, private val end: Int = bytes.size) {
    /** Calls [field] with each field's number and a reader over its value; unknown fields are skipped. */
    fun forEach(field: (Int, Value) -> Unit) {
        while (at < end) {
            val key = varint()
            val value = when ((key and 7).toInt()) {
                0 -> Value(varint(), null)
                1 -> Value(0, null).also { at += 8 }
                5 -> Value(fixed32(), null)
                2 -> varint().toInt().let { n -> Value(0, ProtoReader(bytes, at, at + n)).also { at += n } }
                else -> return // a wire type this reader does not know: stop rather than misread
            }
            field((key ushr 3).toInt(), value)
        }
    }

    private fun varint(): Long {
        var result = 0L
        var shift = 0
        while (true) {
            val b = bytes[at++].toInt()
            result = result or ((b and 0x7f).toLong() shl shift)
            if (b and 0x80 == 0) return result
            shift += 7
        }
    }

    private fun fixed32(): Long {
        var v = 0L
        for (i in 0 until 4) v = v or ((bytes[at + i].toLong() and 0xff) shl (8 * i))
        at += 4
        return v
    }

    fun bytes(): ByteArray = bytes.copyOfRange(at, end)

    class Value(val number: Long, val inner: ProtoReader?) {
        val string get() = inner?.bytes()?.let { String(it) }.orEmpty()
        val hex get() = inner?.bytes()?.joinToString("") { "%02x".format(it) }.orEmpty()
        val float get() = java.lang.Float.intBitsToFloat(number.toInt())
    }
}

class ProtoWriter {
    private val out = ByteArrayOutputStream()

    fun string(field: Int, value: String) = apply {
        if (value.isEmpty()) return@apply
        val bytes = value.toByteArray()
        varint((field shl 3 or 2).toLong())
        varint(bytes.size.toLong())
        out.write(bytes)
    }

    fun uint(field: Int, value: Long) = apply {
        if (value == 0L) return@apply
        varint((field shl 3).toLong())
        varint(value)
    }

    fun message(field: Int, value: ByteArray) = apply {
        varint((field shl 3 or 2).toLong())
        varint(value.size.toLong())
        out.write(value)
    }

    private fun varint(value: Long) {
        var v = value
        while (v >= 0x80) { out.write((v and 0x7f or 0x80).toInt()); v = v ushr 7 }
        out.write(v.toInt())
    }

    fun toByteArray(): ByteArray = out.toByteArray()
}
