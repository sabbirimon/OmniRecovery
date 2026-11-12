package local.omnirecovery.phone

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.OutputStream
import java.io.IOException
import java.io.InputStream
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.PBEKeySpec
import javax.crypto.spec.SecretKeySpec

internal const val MAX_PAYLOAD = 64L * 1024 * 1024 * 1024
internal const val MAX_ARCHIVE = MAX_PAYLOAD + 64L * 1024 * 1024
private const val MAX_BLOCK = 1024 * 1024
private const val MAX_FILES = 20_000

internal data class ArchiveInput(val name: String, val open: () -> InputStream)

internal data class ArchiveSummary(val files: Int, val bytes: Long)

internal interface ArchiveSink {
    fun begin(path: String) {}
    fun write(data: ByteArray, offset: Int, length: Int) {}
    fun end(length: Long, sha256: ByteArray) {}
}

internal object TransferArchive {
    fun read(input: InputStream, password: CharArray, sink: ArchiveSink = object : ArchiveSink {}): ArchiveSummary {
        validate(password.size in 12..128 && password.all { it in ' '..'~' })
        val stream = DataInputStream(input)
        val header = ByteArray(24).also(stream::readFully)
        validate(header.copyOfRange(0, 8).contentEquals("OMNIBK01".toByteArray(Charsets.US_ASCII)))
        val key = key(password, header)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        val paths = HashSet<String>()
        val directories = HashMap<String, String>()
        var sequence = 0L
        var active = false
        var files = 0
        var total = 0L
        var size = 0L
        val digest = MessageDigest.getInstance("SHA-256")
        while (true) {
            val length = stream.readInt()
            validate(length in 1..(MAX_BLOCK + 1))
            val encrypted = ByteArray(length + 16).also(stream::readFully)
            val nonce = ByteBuffer.allocate(12).putInt(0).putLong(sequence).array()
            val aad = ByteBuffer.allocate(36).put(header).putLong(sequence).putInt(length).array()
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, nonce))
            cipher.updateAAD(aad)
            val record = cipher.doFinal(encrypted)
            sequence++
            try {
                val data = ByteBuffer.wrap(record, 1, record.size - 1)
                when (record[0].toInt()) {
                    1 -> {
                        validate(!active && files < MAX_FILES && data.remaining() in 1..1024)
                        val path = Charsets.UTF_8.newDecoder().decode(data).toString()
                        val parts = path.split('/')
                        validate(parts.size <= 64 && parts.all(::portableSegment))
                        val names = parts.map(::asciiFold)
                        val normalized = names.joinToString("/")
                        validate(normalized !in directories && paths.add(normalized))
                        var parent = ""
                        var original = ""
                        for ((index, part) in names.dropLast(1).withIndex()) {
                            parent = if (parent.isEmpty()) part else "$parent/$part"
                            original = if (original.isEmpty()) parts[index] else "$original/${parts[index]}"
                            validate(parent !in paths)
                            val existing = directories.put(parent, original)
                            validate(existing == null || existing == original)
                        }
                        active = true
                        size = 0
                        digest.reset()
                        sink.begin(path)
                    }
                    2 -> {
                        val count = data.remaining()
                        validate(active && count > 0 && count <= MAX_PAYLOAD - total)
                        total += count
                        size += count
                        digest.update(record, 1, count)
                        sink.write(record, 1, count)
                    }
                    3 -> {
                        validate(active && data.remaining() == 40 && data.long == size)
                        val expected = ByteArray(32).also(data::get)
                        validate(MessageDigest.isEqual(expected, digest.digest()))
                        sink.end(size, expected)
                        active = false
                        files++
                    }
                    4 -> {
                        validate(!active && files > 0 && data.remaining() == 12)
                        validate(data.int == files && data.long == total && stream.read() == -1)
                        return ArchiveSummary(files, total)
                    }
                    else -> throw IOException("Unsupported archive record")
                }
            } finally {
                record.fill(0)
            }
        }
    }

    fun write(output: OutputStream, password: CharArray, files: List<ArchiveInput>): ArchiveSummary {
        validate(password.size in 12..128 && password.all { it in ' '..'~' })
        validate(files.size in 1..MAX_FILES)
        val names = HashSet<String>()
        for (file in files) {
            validate(portableSegment(file.name) && names.add(asciiFold(file.name)))
        }
        val header = ByteArray(24).also { SecureRandom().nextBytes(it) }
        "OMNIBK01".toByteArray(Charsets.US_ASCII).copyInto(header)
        val stream = DataOutputStream(output)
        stream.write(header)
        val key = key(password, header)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        var sequence = 0L
        var total = 0L
        fun record(kind: Byte, data: ByteArray, count: Int = data.size) {
            val size = count + 1
            val nonce = ByteBuffer.allocate(12).putInt(0).putLong(sequence).array()
            val aad = ByteBuffer.allocate(36).put(header).putLong(sequence++).putInt(size).array()
            cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(128, nonce))
            cipher.updateAAD(aad)
            val plaintext = ByteArray(size)
            try {
                plaintext[0] = kind
                data.copyInto(plaintext, 1, 0, count)
                stream.writeInt(size)
                stream.write(cipher.doFinal(plaintext))
            } finally {
                plaintext.fill(0)
            }
        }
        val block = ByteArray(MAX_BLOCK)
        try {
            for (file in files) {
                record(1, file.name.toByteArray(Charsets.UTF_8))
                val digest = MessageDigest.getInstance("SHA-256")
                var size = 0L
                file.open().use { input ->
                    while (true) {
                        val count = input.read(block)
                        if (count < 0) break
                        if (count == 0) continue
                        validate(count <= MAX_PAYLOAD - total)
                        size += count
                        total += count
                        digest.update(block, 0, count)
                        record(2, block, count)
                    }
                }
                record(3, ByteBuffer.allocate(40).putLong(size).put(digest.digest()).array())
            }
            record(4, ByteBuffer.allocate(12).putInt(files.size).putLong(total).array())
            stream.flush()
        } finally {
            block.fill(0)
        }
        return ArchiveSummary(files.size, total)
    }

    private fun key(password: CharArray, header: ByteArray): SecretKeySpec {
        val specification = PBEKeySpec(password, header.copyOfRange(8, 24), 600_000, 256)
        val keyBytes = try {
            SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256").generateSecret(specification).encoded
        } finally {
            specification.clearPassword()
        }
        return try { SecretKeySpec(keyBytes, "AES") } finally { keyBytes.fill(0) }
    }

    private fun portableSegment(segment: String): Boolean = segment.isNotEmpty() &&
        segment.toByteArray(Charsets.UTF_8).size <= 255 && !segment.endsWith('.') && !segment.endsWith(' ') && !reservedDeviceName(segment) &&
        segment.none { it.isISOControl() || it in "\\:*?\"<>|" }

    private fun reservedDeviceName(segment: String): Boolean {
        val stem = asciiFold(segment.substringBefore('.').trimEnd(' '))
        if (stem in setOf("con", "prn", "aux", "nul", "conin$", "conout$")) return true
        return (stem.startsWith("com") || stem.startsWith("lpt")) &&
            stem.substring(3) in setOf("1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³")
    }

    private fun asciiFold(value: String): String = buildString(value.length) {
        for (character in value) append(if (character in 'A'..'Z') character.lowercaseChar() else character)
    }

    private fun validate(condition: Boolean) {
        if (!condition) throw IOException("Invalid or unsupported transfer archive")
    }
}
