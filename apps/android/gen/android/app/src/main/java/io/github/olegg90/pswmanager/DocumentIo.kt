package io.github.olegg90.pswmanager

import android.content.Context
import android.net.Uri
import android.os.Environment
import java.io.File
import java.io.FileNotFoundException

/**
 * Documents read and written whole, by their content URI (the database's
 * file when it is a local file, through the Storage Access Framework). Used by
 * [DocumentsPlugin] and, from Rust through JNI, by the background upload.
 */
object DocumentIo {
  /** A file the app reaches by its path (`file://` and the path as it is,
   *  not encoded): a key file browsed with All files access. */
  private fun pathOf(uri: String): File? = if (uri.startsWith("file://")) File(uri.removePrefix("file://")) else null

  /** The content, or null when the document is gone. */
  @JvmStatic
  fun read(context: Context, uri: String): ByteArray? {
    pathOf(uri)?.let { file ->
      if (!Environment.isExternalStorageManager()) {
        throw Exception("All files access is off: turn it on again for PswManager to read ${file.name}")
      }
      return if (file.isFile) file.readBytes() else null
    }
    return try {
      context.contentResolver.openInputStream(Uri.parse(uri))?.use { it.readBytes() }
    } catch (e: FileNotFoundException) {
      null
    }
  }

  @JvmStatic
  fun write(context: Context, uri: String, bytes: ByteArray) {
    // "wt": truncate, so a shorter file leaves nothing of the longer one behind.
    context.contentResolver.openOutputStream(Uri.parse(uri), "wt")?.use { it.write(bytes); it.flush() }
      ?: throw Exception("Cannot write the file")
  }
}
