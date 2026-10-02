package io.github.olegg90.pswmanager

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

/**
 * The background upload (`docs/spec-android.md`, *Synchronisation*): changes
 * the app could not send before Android stopped it go up later, when there is
 * a network. It needs no key: the encrypted working copy is uploaded,
 * conditional on the remote file's revision; if that changed, the merge waits
 * for the next unlock. The work is done in Rust ([BackgroundUpload]).
 */
class UploadWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
  override fun doWork(): Result {
    val state = inputData.getString(STATE) ?: return Result.failure()
    return if (BackgroundUpload.uploadPending(applicationContext, state)) Result.success() else Result.retry()
  }

  companion object {
    private const val STATE = "state"

    /** Asks for the upload of what the app's state file `state` says is waiting;
     *  a cloud store needs a network first. Replaces one asked for before. */
    fun schedule(context: Context, state: String, cloud: Boolean) {
      val network = if (cloud) NetworkType.CONNECTED else NetworkType.NOT_REQUIRED
      val request = OneTimeWorkRequestBuilder<UploadWorker>()
        .setConstraints(Constraints.Builder().setRequiredNetworkType(network).build())
        .setInputData(workDataOf(STATE to state))
        .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
        .build()
      WorkManager.getInstance(context).enqueueUniqueWork("upload", ExistingWorkPolicy.REPLACE, request)
    }
  }
}

/** The app's Rust library, for the background upload. */
object BackgroundUpload {
  init {
    System.loadLibrary("pswm_android_lib")
  }

  /** Uploads what is waiting; true when nothing is left to send. */
  @JvmStatic
  external fun uploadPending(context: Context, state: String): Boolean
}
