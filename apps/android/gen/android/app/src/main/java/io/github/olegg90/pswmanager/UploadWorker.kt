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
  /** Tried again (offline, a failure that may pass) a few times; then the app
   *  sends the changes when it is next opened. */
  override fun doWork(): Result {
    val state = inputData.getString(STATE) ?: return Result.failure()
    val settled = BackgroundUpload.uploadPending(applicationContext, state)
    return if (settled || runAttemptCount >= MAX_ATTEMPTS) Result.success() else Result.retry()
  }

  companion object {
    private const val STATE = "state"
    private const val MAX_ATTEMPTS = 10

    /** Asks for the upload of what the app's state file `state` says is waiting;
     *  a cloud store needs a network first. One asked for before is kept (a
     *  running one is never stopped halfway): it sends whatever is waiting. */
    fun schedule(context: Context, state: String, cloud: Boolean) {
      val network = if (cloud) NetworkType.CONNECTED else NetworkType.NOT_REQUIRED
      val request = OneTimeWorkRequestBuilder<UploadWorker>()
        .setConstraints(Constraints.Builder().setRequiredNetworkType(network).build())
        .setInputData(workDataOf(STATE to state))
        .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
        .build()
      WorkManager.getInstance(context).enqueueUniqueWork("upload", ExistingWorkPolicy.KEEP, request)
    }
  }
}

/** The app's Rust library, for the background upload. */
object BackgroundUpload {
  init {
    System.loadLibrary("pswm_android_lib")
  }

  /** Uploads what is waiting; true when that is settled (sent, nothing to
   *  send, or left for the app), false when it is worth trying again. */
  @JvmStatic
  external fun uploadPending(context: Context, state: String): Boolean
}
