package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.util.Base64
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

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
class OpenArgs {
  /** A file in the app's cache (`cache/open/…`). */
  lateinit var path: String
}

@InvokeArg
class SaveArgs {
  /** The name the save picker suggests. */
  lateinit var name: String
}

@InvokeArg
class FolderArgs {
  /** A folder the user picked ([DocumentsPlugin.pickFolder]). */
  lateinit var folder: String
}

@InvokeArg
class ChildArgs {
  /** A folder the user picked ([DocumentsPlugin.pickFolder]). */
  lateinit var folder: String
  lateinit var name: String
  /** Make it when it is not there; otherwise answer `uri: null`. */
  var create: Boolean = true
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
      // A .kdbx or a key file has no MIME type Android knows.
      .setType("*/*")
      // No starting place: pointed at Documents, some phones open the
      // "Documents" category (files by type), where neither is listed. The
      // picker starts where it was last instead, as the save picker does.
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

  /** A file to read once (to attach it): no lasting access is kept. */
  @Command
  fun pickToRead(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
      .addCategory(Intent.CATEGORY_OPENABLE)
      .setType("*/*")
      .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    startActivityForResult(invoke, intent, "pickedToRead")
  }

  /** The file picked to read, with its size (-1 when the provider does not say); `uri` is null when the user cancelled. */
  @ActivityCallback
  fun pickedToRead(invoke: Invoke, result: ActivityResult) {
    val uri = result.data?.data
    if (result.resultCode != Activity.RESULT_OK || uri == null) {
      invoke.resolve(JSObject().put("uri", null as String?))
      return
    }
    background(invoke) {
      JSObject().put("uri", uri.toString()).put("name", displayName(uri)).put("size", sizeOf(uri))
    }
  }

  /** A new file to save into (an attachment), named by the user in Android's save picker: no lasting access is kept. */
  @Command
  fun pickToSave(invoke: Invoke) {
    val name = invoke.parseArgs(SaveArgs::class.java).name
    val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(File(name).extension.lowercase()) ?: "application/octet-stream"
    val intent = Intent(Intent.ACTION_CREATE_DOCUMENT)
      .addCategory(Intent.CATEGORY_OPENABLE)
      .setType(type)
      .putExtra(Intent.EXTRA_TITLE, name)
      .addFlags(readWrite)
    startActivityForResult(invoke, intent, "pickedToSave")
  }

  /** The file made to save into; `uri` is null when the user cancelled. */
  @ActivityCallback
  fun pickedToSave(invoke: Invoke, result: ActivityResult) {
    val uri = result.data?.data
    invoke.resolve(JSObject().put("uri", if (result.resultCode == Activity.RESULT_OK) uri?.toString() else null))
  }

  /** The names of the files (not folders) in a picked folder. */
  @Command
  fun files(invoke: Invoke) = background(invoke) {
    val folder = Uri.parse(invoke.parseArgs(FolderArgs::class.java).folder)
    val children = DocumentsContract.buildChildDocumentsUriUsingTree(folder, DocumentsContract.getTreeDocumentId(folder))
    val columns = arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE)
    val names = JSArray()
    resolver.query(children, columns, null, null, null)?.use { cursor ->
      while (cursor.moveToNext()) {
        if (cursor.getString(1) != DocumentsContract.Document.MIME_TYPE_DIR) names.put(cursor.getString(0))
      }
    }
    JSObject().put("names", names)
  }

  /** The document named `name` in a picked folder, made when it is not there (unless `create` is false). */
  @Command
  fun child(invoke: Invoke) = background(invoke) {
    val args = invoke.parseArgs(ChildArgs::class.java)
    val folder = Uri.parse(args.folder)
    val uri = findChild(folder, args.name) ?: if (args.create) make(folder, args.name) else null
    JSObject().put("uri", uri?.toString())
  }

  /** The content, base64; `data` is null when the document is gone. */
  @Command
  fun read(invoke: Invoke) = background(invoke) {
    val bytes = DocumentIo.read(activity, invoke.parseArgs(DocumentArgs::class.java).uri)
    JSObject().put("data", bytes?.let { Base64.encodeToString(it, Base64.NO_WRAP) })
  }

  @Command
  fun write(invoke: Invoke) = background(invoke) {
    val args = invoke.parseArgs(WriteArgs::class.java)
    DocumentIo.write(activity, args.uri, Base64.decode(args.data, Base64.NO_WRAP))
    JSObject()
  }

  /** Hands a file in the app's cache to another app, read only, through the FileProvider. */
  @Command
  fun openFile(invoke: Invoke) {
    try {
      val file = File(invoke.parseArgs(OpenArgs::class.java).path)
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
      val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      activity.startActivity(Intent.createChooser(view, file.name))
      invoke.resolve(JSObject())
    } catch (e: Exception) {
      invoke.reject("Cannot open the file: ${e.message}")
    }
  }

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

  private fun sizeOf(uri: Uri): Long {
    resolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst() && !cursor.isNull(0)) return cursor.getLong(0)
    }
    return -1
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
