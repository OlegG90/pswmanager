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
import java.io.FileNotFoundException

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

  /** The picker starts in Documents on the phone's own storage. */
  private val documentsFolder: Uri =
    DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", "primary:Documents")

  @Command
  fun pickFolder(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
      .putExtra(DocumentsContract.EXTRA_INITIAL_URI, documentsFolder)
      .addFlags(readWrite or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    startActivityForResult(invoke, intent, "picked")
  }

  @Command
  fun pickFile(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
      .addCategory(Intent.CATEGORY_OPENABLE)
      // A .kdbx has no MIME type Android knows.
      .setType("*/*")
      .putExtra(DocumentsContract.EXTRA_INITIAL_URI, documentsFolder)
      .addFlags(readWrite or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    startActivityForResult(invoke, intent, "picked")
  }

  /** The folder or file picked, its permission kept; `uri` is null when the user cancelled. */
  @ActivityCallback
  fun picked(invoke: Invoke, result: ActivityResult) {
    val uri = result.data?.data
    if (result.resultCode != Activity.RESULT_OK || uri == null) {
      invoke.resolve(JSObject().put("uri", null as String?))
      return
    }
    background(invoke) {
      resolver.takePersistableUriPermission(uri, readWrite)
      JSObject().put("uri", uri.toString()).put("name", displayName(uri))
    }
  }

  /** The document named `name` in a picked folder, made when it is not there. */
  @Command
  fun child(invoke: Invoke) = background(invoke) {
    val args = invoke.parseArgs(ChildArgs::class.java)
    val folder = Uri.parse(args.folder)
    val uri = findChild(folder, args.name) ?: make(folder, args.name)
    JSObject().put("uri", uri.toString())
  }

  /** The content, base64; `data` is null when the document is gone. */
  @Command
  fun read(invoke: Invoke) = background(invoke) {
    val bytes = try {
      resolver.openInputStream(uriOf(invoke))?.use { it.readBytes() }
    } catch (e: FileNotFoundException) {
      null
    }
    JSObject().put("data", bytes?.let { Base64.encodeToString(it, Base64.NO_WRAP) })
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

  private fun uriOf(invoke: Invoke): Uri = Uri.parse(invoke.parseArgs(DocumentArgs::class.java).uri)

  private fun folderDocument(folder: Uri): Uri =
    DocumentsContract.buildDocumentUriUsingTree(folder, DocumentsContract.getTreeDocumentId(folder))

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

  /** A new, empty document named exactly `name` (a provider may add an extension for the type: renamed back). */
  private fun make(folder: Uri, name: String): Uri {
    val made = DocumentsContract.createDocument(resolver, folderDocument(folder), "application/octet-stream", name)
      ?: throw Exception("Cannot make $name in the folder")
    if (displayName(made) == name) return made
    return DocumentsContract.renameDocument(resolver, made, name) ?: made
  }

  private fun displayName(uri: Uri): String {
    val document = if (DocumentsContract.isTreeUri(uri) && !DocumentsContract.isDocumentUri(activity, uri)) folderDocument(uri) else uri
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
