package local.omnirecovery.phone

import android.app.Activity
import android.content.ContentResolver
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.text.InputType
import android.view.View
import android.webkit.MimeTypeMap
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.io.IOException
import java.io.OutputStream
import java.security.MessageDigest
import java.util.Locale
import java.util.concurrent.Executors

class MainActivity : Activity() {
    private val executor = Executors.newSingleThreadExecutor()
    private var archive: Uri? = null
    private var destination: Uri? = null
    private var backupFiles: List<Uri> = emptyList()
    private lateinit var password: EditText
    private lateinit var status: TextView
    private lateinit var pickArchive: Button
    private lateinit var pickDestination: Button
    private lateinit var restore: Button
    private lateinit var verify: Button
    private lateinit var pickBackupFiles: Button
    private lateinit var createBackup: Button
    private lateinit var backupPassword: EditText
    private lateinit var backupConfirm: EditText
    private lateinit var progress: ProgressBar
    private var running = false

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        val canvas = ScrollView(this).apply { setBackgroundColor(BACKGROUND) }
        val body = column().apply { setPadding(dp(20), dp(20), dp(20), dp(28)) }
        canvas.addView(body)
        canvas.setOnApplyWindowInsetsListener { view, insets ->
            @Suppress("DEPRECATION")
            view.setPadding(insets.systemWindowInsetLeft, insets.systemWindowInsetTop, insets.systemWindowInsetRight, insets.systemWindowInsetBottom)
            insets
        }
        setContentView(canvas)

        val header = column().apply {
            background = rounded(NAVY)
            setPadding(dp(20), dp(20), dp(20), dp(20))
            addView(label("OmniRecovery", 15f, Color.rgb(176, 205, 255), true))
            addView(label("Backup & restore", 28f, Color.WHITE, true), row(top = 7))
            addView(label("Create an encrypted file backup or restore an archive from a computer or another phone.", 15f, Color.WHITE), row(top = 7))
        }
        body.addView(header, row())
        val sourceCard = card("Backup archive")
        pickArchive = action("Choose .omnibackup file", false) { chooseArchive() }
        sourceCard.addView(pickArchive, row(top = 10))
        body.addView(sourceCard, row(top = 14))

        val targetCard = card("Restore destination")
        targetCard.addView(label("OmniRecovery creates a new folder in the location you choose.", 14f, MUTED), row(top = 4))
        pickDestination = action("Choose destination folder", false) { chooseDestination() }
        targetCard.addView(pickDestination, row(top = 10))
        body.addView(targetCard, row(top = 12))

        val verifyCard = card("Verify and restore")
        password = EditText(this).apply {
            hint = "Passphrase for encrypted archives"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            isSingleLine = true
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
        }
        verifyCard.addView(label("Leave the passphrase blank for an unencrypted transfer.", 14f, MUTED), row(top = 4))
        verifyCard.addView(password, row(top = 7))
        verify = action("Verify without restoring", false) { startRestore(true) }
        verifyCard.addView(verify, row(top = 8))
        restore = action("Verify and restore files", true) { startRestore(false) }
        verifyCard.addView(restore, row(top = 12))
        progress = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
            isIndeterminate = true
            indeterminateTintList = ColorStateList.valueOf(BLUE)
            visibility = View.GONE
        }
        verifyCard.addView(progress, row(top = 10))
        status = label("Choose an archive and destination to begin.", 14f, MUTED).apply {
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        verifyCard.addView(status, row(top = 9))
        body.addView(verifyCard, row(top = 12))

        val backupCard = card("Create a phone file backup")
        backupCard.addView(label("Select photos, videos, or other files. The desktop app can restore the resulting archive.", 14f, MUTED), row(top = 4))
        pickBackupFiles = action("Choose files", false) { chooseBackupFiles() }
        backupCard.addView(pickBackupFiles, row(top = 8))
        backupPassword = passphraseField("New backup passphrase")
        backupConfirm = passphraseField("Repeat passphrase")
        backupCard.addView(backupPassword, row(top = 7))
        backupCard.addView(backupConfirm, row(top = 7))
        createBackup = action("Save encrypted backup", true) { chooseBackupOutput() }
        backupCard.addView(createBackup, row(top = 10))
        body.addView(backupCard, row(top = 12))
        updateControls()
    }

    @Suppress("DEPRECATION")
    private fun chooseArchive() {
        startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_LOCAL_ONLY, true)
        }, PICK_ARCHIVE)
    }

    @Suppress("DEPRECATION")
    private fun chooseDestination() {
        startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            putExtra(Intent.EXTRA_LOCAL_ONLY, true)
        }, PICK_DESTINATION)
    }

    private fun passphraseField(title: String) = EditText(this).apply {
        hint = title
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        isSingleLine = true
        importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
    }

    @Suppress("DEPRECATION")
    private fun chooseBackupFiles() {
        startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
            putExtra(Intent.EXTRA_LOCAL_ONLY, true)
        }, PICK_BACKUP_FILES)
    }

    @Suppress("DEPRECATION")
    private fun chooseBackupOutput() {
        val valid = backupPassword.length() in 12..128 &&
            (0 until backupPassword.length()).all { backupPassword.text[it] in ' '..'~' }
        val matching = backupPassword.length() == backupConfirm.length() &&
            (0 until backupPassword.length()).all { backupPassword.text[it] == backupConfirm.text[it] }
        if (!valid || !matching) {
            backupConfirm.error = "Enter matching 12–128 character passphrases."
            return
        }
        startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/octet-stream"
            putExtra(Intent.EXTRA_TITLE, "OmniRecovery-phone.omnibackup")
            putExtra(Intent.EXTRA_LOCAL_ONLY, true)
        }, CREATE_BACKUP)
    }

    @Deprecated("Uses platform document pickers without an AndroidX dependency")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) return
        if (requestCode == PICK_BACKUP_FILES) {
            backupFiles = data?.clipData?.let { clip ->
                (0 until minOf(clip.itemCount, 1025)).map { clip.getItemAt(it).uri }.distinct()
            } ?: listOfNotNull(data?.data)
            if (backupFiles.size > 1024) backupFiles = emptyList()
            pickBackupFiles.text = if (backupFiles.isEmpty()) "Choose up to 1,024 files" else "${backupFiles.size} files selected"
            updateControls()
            return
        }
        val uri = data?.data ?: return
        when (requestCode) {
            CREATE_BACKUP -> startBackup(uri)
            PICK_ARCHIVE -> {
                archive = uri
                pickArchive.text = displayName(uri) ?: "Archive selected"
            }
            PICK_DESTINATION -> {
                if (!DocumentsContract.isTreeUri(uri)) return
                destination = uri
                pickDestination.text = "Destination selected"
            }
        }
        updateControls()
    }

    private fun startRestore(verifyOnly: Boolean) {
        val source = archive ?: return
        val tree = if (verifyOnly) null else destination ?: return
        val passphrase = CharArray(password.length()) { password.text[it] }
        password.text.clear()
        running = true
        status.text = "Verifying archive."
        updateControls()
        executor.execute {
            var folder: Uri? = null
            var cached: File? = null
            val message = try {
                val file = File.createTempFile("archive-", ".omnibackup", cacheDir)
                cached = file
                cacheArchive(source, file)
                val verified = file.inputStream().buffered().use { TransferArchive.read(it, passphrase) }
                if (tree == null) {
                    "Verified ${verified.files} files and ${verified.bytes} bytes. No restore was performed."
                } else {
                    runOnUiThread { status.text = "Archive verified. Restoring ${verified.files} files."
                    }
                    val parent = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
                    val fresh = create(parent, DocumentsContract.Document.MIME_TYPE_DIR, "OmniRecovery Restore - incomplete")
                    folder = fresh
                    PhoneSink(contentResolver, fresh).use { sink ->
                        val restored = file.inputStream().buffered().use { TransferArchive.read(it, passphrase, sink) }
                        check(restored == verified)
                    }
                    val renamed = runCatching { DocumentsContract.renameDocument(contentResolver, fresh, "OmniRecovery Restore") }.getOrNull()
                    val name = displayName(renamed ?: fresh) ?: "the new destination folder"
                    "Restored and verified ${verified.files} files in $name."
                }
            } catch (error: ArchivePassphraseError) {
                error.message ?: "Check the archive passphrase."
            } catch (_: Exception) {
                if (folder == null) "Restore stopped before writing files. Check the passphrase, archive and available space."
                else "Restore stopped. The new folder may contain partial results. Existing folders were left in place."
            } finally {
                passphrase.fill('\u0000')
                cached?.delete()
            }
            runOnUiThread {
                if (!isDestroyed) {
                    running = false
                    status.text = message
                    updateControls()
                }
            }
        }
    }

    private fun startBackup(output: Uri) {
        val files = backupFiles
        val passphrase = CharArray(backupPassword.length()) { backupPassword.text[it] }
        backupPassword.text.clear()
        backupConfirm.text.clear()
        running = true
        createBackup.text = "Creating and verifying backup…"
        updateControls()
        executor.execute {
            var cached: File? = null
            val message = try {
                val inputs = files.map { uri ->
                    ArchiveInput(displayName(uri) ?: throw IOException("Missing filename")) {
                        contentResolver.openInputStream(uri) ?: throw IOException("Could not open selected file")
                    }
                }
                val file = File.createTempFile("backup-", ".encrypted", cacheDir)
                cached = file
                val result = file.outputStream().buffered().use { TransferArchive.write(it, passphrase, inputs) }
                check(file.inputStream().buffered().use { TransferArchive.read(it, passphrase) } == result)
                val fileDigest = MessageDigest.getInstance("SHA-256")
                val digest = file.inputStream().use { source ->
                    val buffer = ByteArray(1024 * 1024)
                    val target = contentResolver.openOutputStream(output, "w") ?: throw IOException("Could not open the chosen destination")
                    target.use { sink ->
                        while (true) {
                            val count = source.read(buffer)
                            if (count < 0) break
                            fileDigest.update(buffer, 0, count)
                            sink.write(buffer, 0, count)
                        }
                    }
                    fileDigest.digest()
                }
                val stored = contentResolver.openInputStream(output) ?: throw IOException("Could not recheck the saved backup")
                val storedDigest = stored.use { source ->
                    val buffer = ByteArray(1024 * 1024)
                    while (true) {
                        val count = source.read(buffer)
                        if (count < 0) break
                        fileDigest.update(buffer, 0, count)
                    }
                    fileDigest.digest()
                }
                check(MessageDigest.isEqual(digest, storedDigest))
                "Saved and verified ${result.files} files."
            } catch (_: Exception) {
                runCatching { DocumentsContract.deleteDocument(contentResolver, output) }
                "Backup stopped. Check selected names, access, and available space."
            } finally {
                passphrase.fill('\u0000')
                cached?.delete()
            }
            runOnUiThread {
                if (!isDestroyed) {
                    running = false
                    createBackup.text = "Save encrypted backup"
                    status.text = message
                    updateControls()
                }
            }
        }
    }

    private fun cacheArchive(uri: Uri, target: File) {
        val input = contentResolver.openInputStream(uri) ?: throw IOException("Cannot open archive")
        input.use { source ->
            target.outputStream().use { output ->
                val block = ByteArray(1024 * 1024)
                var total = 0L
                while (true) {
                    val count = source.read(block)
                    if (count < 0) break
                    total += count
                    if (total > MAX_ARCHIVE) throw IOException("Archive exceeds the size limit")
                    output.write(block, 0, count)
                }
            }
        }
    }

    private fun updateControls() {
        pickArchive.isEnabled = !running
        pickDestination.isEnabled = !running
        password.isEnabled = !running
        restore.isEnabled = !running && archive != null && destination != null
        verify.isEnabled = !running && archive != null
        pickBackupFiles.isEnabled = !running
        backupPassword.isEnabled = !running
        backupConfirm.isEnabled = !running
        createBackup.isEnabled = !running && backupFiles.isNotEmpty()
        progress.visibility = if (running) View.VISIBLE else View.GONE
    }

    private fun create(parent: Uri, mime: String, name: String): Uri =
        DocumentsContract.createDocument(contentResolver, parent, mime, name)
            ?: throw IOException("Could not create a new restore folder")

    private fun displayName(uri: Uri): String? = runCatching {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    }.getOrNull()

    override fun onDestroy() {
        executor.shutdown()
        super.onDestroy()
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()
    private fun column() = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
    private fun row(top: Int = 0) = LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(top) }
    private fun rounded(color: Int) = GradientDrawable().apply { setColor(color); cornerRadius = dp(14).toFloat() }
    private fun label(value: String, size: Float, color: Int, bold: Boolean = false) = TextView(this).apply {
        text = value
        textSize = size
        setTextColor(color)
        typeface = if (bold) Typeface.DEFAULT_BOLD else Typeface.DEFAULT
    }
    private fun card(title: String) = column().apply {
        background = rounded(Color.WHITE).apply { setStroke(dp(1), Color.rgb(217, 228, 243)) }
        setPadding(dp(16), dp(14), dp(16), dp(14))
        addView(label(title, 18f, NAVY, true))
    }
    private fun action(title: String, primary: Boolean, listener: () -> Unit) = Button(this).apply {
        text = title
        isAllCaps = false
        minHeight = dp(48)
        setTextColor(if (primary) Color.WHITE else NAVY)
        backgroundTintList = ColorStateList.valueOf(if (primary) BLUE else Color.rgb(228, 237, 254))
        setOnClickListener { listener() }
    }

    private companion object {
        const val PICK_ARCHIVE = 10
        const val PICK_DESTINATION = 11
        const val PICK_BACKUP_FILES = 12
        const val CREATE_BACKUP = 13
        val BACKGROUND = Color.rgb(242, 247, 252)
        val NAVY = Color.rgb(20, 45, 82)
        val BLUE = Color.rgb(34, 87, 207)
        val MUTED = Color.rgb(73, 91, 115)
    }
}

private class PhoneSink(private val resolver: ContentResolver, root: Uri) : ArchiveSink, AutoCloseable {
    private val folders = mutableMapOf("" to root)
    private var file: Uri? = null
    private var output: OutputStream? = null

    override fun begin(path: String) {
        val names = path.split('/')
        var directory = ""
        for (name in names.dropLast(1)) {
            val parent = folders.getValue(directory)
            directory = if (directory.isEmpty()) name else "$directory/$name"
            folders.getOrPut(directory) { createExact(parent, DocumentsContract.Document.MIME_TYPE_DIR, name) }
        }
        val name = names.last()
        val extension = name.substringAfterLast('.', "").lowercase(Locale.ROOT)
        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension) ?: "application/octet-stream"
        val uri = createExact(folders.getValue(directory), mime, name)
        file = uri
        output = resolver.openOutputStream(uri, "w")?.buffered() ?: throw IOException("Cannot write restored file")
    }

    override fun write(data: ByteArray, offset: Int, length: Int) {
        checkNotNull(output).write(data, offset, length)
    }

    override fun end(length: Long, sha256: ByteArray) {
        close()
        val uri = checkNotNull(file)
        val input = resolver.openInputStream(uri) ?: throw IOException("Cannot verify restored file")
        val digest = MessageDigest.getInstance("SHA-256")
        input.use {
            val buffer = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
                val count = it.read(buffer)
                if (count < 0) break
                total += count
                if (total > length) throw IOException("Restored file has unexpected length")
                digest.update(buffer, 0, count)
            }
            if (total != length || !MessageDigest.isEqual(digest.digest(), sha256)) {
                throw IOException("Restored file did not verify")
            }
        }
        file = null
    }

    private fun createExact(parent: Uri, mime: String, name: String): Uri {
        val created = DocumentsContract.createDocument(resolver, parent, mime, name)
            ?: throw IOException("Cannot create restore destination entry")
        val actual = resolver.query(created, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
        if (actual != name) throw IOException("The selected provider changed a restore name")
        return created
    }

    override fun close() {
        val stream = output
        output = null
        stream?.close()
    }
}
