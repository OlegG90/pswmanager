package io.github.olegg90.pswmanager

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.Gravity
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Reads a TOTP QR code with the camera (#280): the preview, and each frame's
 * brightness handed to ZXing in memory, never saved. The first code holding
 * an `otpauth:` URI is the result ([TEXT]); the core checks it. The screen
 * cannot be captured whatever the settings say: it shows the secret's code.
 * The camera permission is asked the first time; refused, the result says
 * why ([ERROR]).
 */
class ScanActivity : AppCompatActivity() {
  companion object {
    const val TEXT = "text"
    const val ERROR = "error"
  }

  private lateinit var analysis: ExecutorService
  private var done = false

  private val permission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
    if (granted) start() else fail("PswManager may not use the camera: allow it in Android's settings, or pick a picture")
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    analysis = Executors.newSingleThreadExecutor()
    if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
      start()
    } else {
      permission.launch(Manifest.permission.CAMERA)
    }
  }

  override fun onDestroy() {
    super.onDestroy()
    analysis.shutdown()
  }

  private fun start() {
    val preview = PreviewView(this)
    val hint = TextView(this).apply {
      text = "Point the camera at the QR code"
      setTextColor(0xFFFFFFFF.toInt())
      setShadowLayer(4f, 0f, 0f, 0xFF000000.toInt())
      textSize = 18f
      setPadding(32, 48, 32, 48)
    }
    setContentView(FrameLayout(this).apply {
      addView(preview)
      addView(hint, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL))
    })
    val providing = ProcessCameraProvider.getInstance(this)
    providing.addListener({
      val camera = try {
        providing.get()
      } catch (e: Exception) {
        return@addListener fail("The camera cannot be opened: ${e.message}")
      }
      val shown = Preview.Builder().build().also { it.setSurfaceProvider(preview.surfaceProvider) }
      val frames = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
      val reader = MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))) }
      frames.setAnalyzer(analysis) { frame -> read(reader, frame)?.let { runOnUiThread { found(it) } } }
      try {
        camera.unbindAll()
        camera.bindToLifecycle(this, CameraSelector.DEFAULT_BACK_CAMERA, shown, frames)
      } catch (e: Exception) {
        fail("The camera cannot be opened: ${e.message}")
      }
    }, ContextCompat.getMainExecutor(this))
  }

  /** The `otpauth:` text of a QR code in `frame`, or null; the frame's copy is wiped. */
  private fun read(reader: MultiFormatReader, frame: ImageProxy): String? {
    frame.use {
      val plane = it.planes[0]
      val luma = ByteArray(plane.buffer.remaining()).also { bytes -> plane.buffer.get(bytes) }
      try {
        val source = PlanarYUVLuminanceSource(luma, plane.rowStride, it.height, 0, 0, it.width, it.height, false)
        val text = reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
        return text.takeIf { t -> t.regionMatches(0, "otpauth:", 0, 8, ignoreCase = true) }
      } catch (_: Exception) {
        return null // no code in this frame (NotFoundException), or one ZXing cannot read
      } finally {
        reader.reset()
        luma.fill(0)
      }
    }
  }

  private fun found(text: String) {
    if (done) return
    done = true
    setResult(RESULT_OK, Intent().putExtra(TEXT, text))
    finish()
  }

  private fun fail(why: String) {
    if (done) return
    done = true
    setResult(RESULT_CANCELED, Intent().putExtra(ERROR, why))
    finish()
  }
}
