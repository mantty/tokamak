package com.tokamak.runtime

import android.app.Activity
import android.content.res.AssetManager
import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.Drawable
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewTreeObserver
import android.widget.FrameLayout
import android.widget.ImageView
import kotlin.math.roundToInt

/**
 * The app's splash screen: its image centred on its background colour, shown
 * over the WebView from launch until the app's first page has loaded. The
 * build names the images, width and colours in the manifest's meta-data.
 */
internal class TokamakSplash private constructor(private val view: View) {
    fun remove() {
        (view.parent as? ViewGroup)?.removeView(view)
    }

    companion object {
        /** Shows the splash `metadata` describes over `activity`, or returns null when the app has none. */
        fun show(activity: Activity, metadata: Bundle?): TokamakSplash? {
            if (metadata?.containsKey("tokamak.splash.width") != true) return null
            val suffix = if (activity.resources.configuration.isNightModeActive) "-dark" else ""
            val view = FrameLayout(activity).apply {
                setBackgroundColor(metadata.getInt("tokamak.splash.background$suffix"))
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
                // Touches stop here rather than reaching the page beneath.
                isClickable = true
            }
            activity.addContentView(
                view,
                ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT),
            )
            val image = metadata.getString("tokamak.splash.image$suffix")
            if (image != null) {
                val density = activity.resources.displayMetrics.density
                addImage(activity, view, image, (metadata.getInt("tokamak.splash.width") * density).roundToInt())
            }
            return TokamakSplash(view)
        }

        /**
         * Decodes the asset `image` `width` pixels wide on a worker thread and
         * centres it in `view`. The window's first frame waits for the decode,
         * so it shows the image, and Android 12 and later keeps its system
         * splash until then.
         */
        private fun addImage(activity: Activity, view: FrameLayout, image: String, width: Int) {
            var decoded = false
            val content = activity.findViewById<View>(android.R.id.content)
            content.viewTreeObserver.addOnPreDrawListener(object : ViewTreeObserver.OnPreDrawListener {
                override fun onPreDraw(): Boolean {
                    if (decoded) content.viewTreeObserver.removeOnPreDrawListener(this)
                    return decoded
                }
            })
            Thread({
                val drawable = runCatching { decode(activity.assets, image, width) }
                    .onFailure { Log.w("tokamak", "tokamak could not decode the splash image $image", it) }
                    .getOrNull()
                activity.runOnUiThread {
                    decoded = true
                    if (drawable == null) return@runOnUiThread
                    val imageView = ImageView(activity).apply {
                        adjustViewBounds = true
                        setImageDrawable(drawable)
                    }
                    view.addView(imageView, FrameLayout.LayoutParams(width, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER))
                    // A GIF plays with the loop count stored in the file.
                    (drawable as? AnimatedImageDrawable)?.start()
                }
            }, "tokamak-splash").start()
        }

        /** The asset `image`, decoded `width` pixels wide at its aspect ratio. */
        private fun decode(assets: AssetManager, image: String, width: Int): Drawable =
            ImageDecoder.decodeDrawable(ImageDecoder.createSource(assets, image)) { decoder, info, _ ->
                val height = width.toLong() * info.size.height / info.size.width
                decoder.setTargetSize(width, height.toInt().coerceAtLeast(1))
            }
    }
}
