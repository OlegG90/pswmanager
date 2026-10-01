package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.util.Base64
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class DocumentArgs {
  lateinit var uri: String
}

@InvokeArg
class WriteArgs {
  lateinit var uri: String
  /** The content, base64. */
  lateinit var data: String
}

@InvokeArg
class ChildArgs {
  /** A folder the user picked ([DocumentsPlugin.pickFolder]). */
  lateinit var folder: String
  lateinit var name: String
}

/**
 * Files the app reaches through Android's Storage Access Framework: a folder or
 * a file the user picks (the permission is kept, so it is asked once), and
 * documents read and written whole. Reads and writes run off the main thread.
 */
@TauriPlugin
class DocumentsPlugin(private val activity: Activity) : Plugin(activity) {
  private val resolver get() = activity.contentResolver

  private val readWrite = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION

  @Command
  fun pickFolder(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(readWrite or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    startActivityForResult(invoke, intent, "picked")
  }

  @Command
  fun pickFile(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
      .addCategory(Intent.CATEGORY_OPENABLE)
      // A .kdbx has no MIME type Android knows.
      .setType("*/*")
      .addFlags(readWrite or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    startActivityForResult(invoke, intent, "picked")
  }

  /** The folder or file picked, its permission kept; `uri` is null when the user cancelled. */
  @ActivityCallback
  fun picked(invoke: Invoke, result: ActivityResult) {
    val uri = result.data?.data
    val answer = JSObject()
    if (result.resultCode != Activity.RESULT_OK || uri == null) {
      answer.put("uri", null as String?)
      invoke.resolve(answer)
      return
    }
    try {
      resolver.takePersistableUriPermission(uri, readWrite)
      answer.put("uri", uri.toString())
      answer.put("name", displayName(uri))
      invoke.resolve(answer)
    } catch (e: Exception) {
      invoke.reject("Cannot keep access to it: ${e.message}")
    }
  }

  /** The document named `name` in a picked folder, made when it is not there. */
  @Command
  fun child(invoke: Invoke) = background(invoke) {
    val args = invoke.parseArgs(ChildArgs::class.java)
    val folder = Uri.parse(args.folder)
    val folderDoc = DocumentsContract.buildDocumentUriUsingTree(folder, DocumentsContract.getTreeDocumentId(folder))
    val found = findChild(folder, args.name)
    val uri = found
      ?: DocumentsContract.createDocument(resolver, folderDoc, "application/octet-stream", args.name)
      ?: throw Exception("Cannot make ${args.name} in the folder")
    JSObject().put("uri", uri.toString()).put("made", found == null)
  }

  @Command
  fun stamp(invoke: Invoke) = background(invoke) {
    val uri = Uri.parse(invoke.parseArgs(DocumentArgs::class.java).uri)
    val columns = arrayOf(DocumentsContract.Document.COLUMN_LAST_MODIFIED, DocumentsContract.Document.COLUMN_SIZE)
    resolver.query(uri, columns, null, null, null)?.use { cursor ->
      if (!cursor.moveToFirst()) throw Exception("The file is gone")
      JSObject().put("modified", cursor.getLong(0)).put("size", cursor.getLong(1))
    } ?: throw Exception("The file is gone")
  }

  @Command
  fun read(invoke: Invoke) = background(invoke) {
    val uri = Uri.parse(invoke.parseArgs(DocumentArgs::class.java).uri)
    val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: throw Exception("Cannot open the file")
    JSObject().put("data", Base64.encodeToString(bytes, Base64.NO_WRAP))
  }

  @Command
  fun write(invoke: Invoke) = background(invoke) {
    val args = invoke.parseArgs(WriteArgs::class.java)
    val bytes = Base64.decode(args.data, Base64.NO_WRAP)
    // "wt": truncate, so a shorter file leaves nothing of the longer one behind.
    resolver.openOutputStream(Uri.parse(args.uri), "wt")?.use { it.write(bytes); it.flush() }
      ?: throw Exception("Cannot write the file")
    JSObject()
  }

  private fun findChild(folder: Uri, name: String): Uri? {
    val children = DocumentsContract.buildChildDocumentsUriUsingTree(folder, DocumentsContract.getTreeDocumentId(folder))
    val columns = arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME)
    resolver.query(children, columns, null, null, null)?.use { cursor ->
      while (cursor.moveToNext()) {
        if (cursor.getString(1) == name) return DocumentsContract.buildDocumentUriUsingTree(folder, cursor.getString(0))
      }
    }
    return null
  }

  private fun displayName(uri: Uri): String {
    val document = if (DocumentsContract.isTreeUri(uri) && !DocumentsContract.isDocumentUri(activity, uri)) {
      DocumentsContract.buildDocumentUriUsingTree(uri, DocumentsContract.getTreeDocumentId(uri))
    } else uri
    resolver.query(document, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) return cursor.getString(0)
    }
    return uri.lastPathSegment ?: ""
  }

  /** Runs `work` off the main thread and answers with what it returns, or its error. */
  private fun background(invoke: Invoke, work: () -> JSObject) {
    Thread {
      try {
        invoke.resolve(work())
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }.start()
  }
}
