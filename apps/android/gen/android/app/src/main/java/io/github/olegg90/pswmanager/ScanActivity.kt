package io.github.olegg90.pswmanager

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Size
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
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
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
 * brightness handed to ZXing in memory, never saved. The first QR code read
 * is the result ([TEXT]); the core checks it as it checks a picture's, so a
 * code holding something else is explained rather than passed over. The
 * screen cannot be captured whatever *Allow screenshots* says ([Screenshots]
 * is not used): it shows the secret's code. The camera permission is asked
 * the first time, with what it is for on the screen behind the question;
 * refused, the result says why ([ERROR]).
 */
class ScanActivity : AppCompatActivity() {
  companion object {
    const val TEXT = "text"
    const val ERROR = "error"
    private const val CANNOT_OPEN = "The camera cannot be opened"
  }

  private val analysis: ExecutorService = Executors.newSingleThreadExecutor()
  private val preview by lazy { PreviewView(this) }
  private var frames: ImageAnalysis? = null
  /** A result is given: frames still queued are not decoded. */
  @Volatile private var done = false
  /** The frame's brightness, reused from frame to frame (on the analysis thread). */
  private var luma = ByteArray(0)

  private val permission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
    if (granted) start() else fail("PswManager may not use the camera: allow it in Android's settings, or pick a picture")
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    val hint = TextView(this).apply {
      text = "Point the camera at the QR code. Only the code is read; no picture is kept."
      setTextColor(0xFFFFFFFF.toInt())
      setShadowLayer(4f, 0f, 0f, 0xFF000000.toInt())
      textSize = 18f
      gravity = Gravity.CENTER
      setPadding(32, 48, 32, 48)
    }
    setContentView(FrameLayout(this).apply {
      addView(preview)
      addView(hint, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM))
    })
    when {
      ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED -> start()
      // Recreated while the question is up (a foldable unfolded, say): its
      // answer still comes to this one.
      savedInstanceState == null -> permission.launch(Manifest.permission.CAMERA)
    }
  }

  override fun onDestroy() {
    frames?.clearAnalyzer()
    analysis.shutdown()
    super.onDestroy()
  }

  private fun start() {
    val providing = ProcessCameraProvider.getInstance(this)
    providing.addListener({
      val camera = try {
        providing.get()
      } catch (e: Exception) {
        return@addListener fail("$CANNOT_OPEN: ${e.message}")
      }
      val shown = Preview.Builder().build().also { it.setSurfaceProvider(preview.surfaceProvider) }
      // Well above the default 640×480, so a dense code shown on another
      // screen still has a few pixels to each of its modules.
      val size = ResolutionSelector.Builder()
        .setResolutionStrategy(ResolutionStrategy(Size(1920, 1080), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER))
        .build()
      val analyzed = ImageAnalysis.Builder()
        .setResolutionSelector(size)
        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
        .build()
      // Trying harder: an export's code (#281) is dense, and shown on another
      // screen it reaches the camera with moiré and glare.
      val hints = mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true)
      val reader = MultiFormatReader().apply { setHints(hints) }
      analyzed.setAnalyzer(analysis) { frame -> read(reader, frame)?.let { runOnUiThread { found(it) } } }
      frames = analyzed
      try {
        camera.unbindAll()
        camera.bindToLifecycle(this, CameraSelector.DEFAULT_BACK_CAMERA, shown, analyzed)
      } catch (e: Exception) {
        fail("$CANNOT_OPEN: ${e.message}")
      }
    }, ContextCompat.getMainExecutor(this))
  }

  /** The text of a QR code in `frame`, or null; the frame's copy is wiped. */
  private fun read(reader: MultiFormatReader, frame: ImageProxy): String? = frame.use {
    if (done) return@use null
    val plane = it.planes[0]
    if (luma.size != plane.buffer.remaining()) luma = ByteArray(plane.buffer.remaining())
    plane.buffer.get(luma)
    try {
      val source = PlanarYUVLuminanceSource(luma, plane.rowStride, it.height, 0, 0, it.width, it.height, false)
      reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
    } catch (_: Exception) {
      null // no code in this frame (NotFoundException), or one ZXing cannot read
    } finally {
      reader.reset()
      luma.fill(0)
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
