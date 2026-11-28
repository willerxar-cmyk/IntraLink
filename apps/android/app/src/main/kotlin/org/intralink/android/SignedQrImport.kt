package org.intralink.android

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.CancellationSignal
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.multi.qrcode.QRCodeMultiReader
import java.io.ByteArrayOutputStream
import java.io.InputStream

/** Explicitly selected local image only. No camera, background clipboard, or
 * automatic pairing. The shared Rust parser still validates the signed payload. */
internal object SignedQrImport {
    private const val MAX_BYTES=8*1024*1024
    private const val MAX_SOURCE=4096
    private const val MAX_DECODE=2048
    private const val MAX_PIXELS=MAX_DECODE*MAX_DECODE
    private const val MAX_CODE=2048

    fun read(context: Context, uri: Uri, cancellation: CancellationSignal): String {
        require(uri.scheme=="content")
        val bytes=context.contentResolver.openAssetFileDescriptor(uri,"r",cancellation)?.use { descriptor ->
            require(descriptor.length<0 || descriptor.length<=MAX_BYTES)
            descriptor.createInputStream().use { bounded(it,cancellation) }
        } ?: error("Cannot open selected image")
        cancellation.throwIfCanceled()
        return decodeBytes(bytes,cancellation)
    }

    private fun bounded(stream: InputStream, cancellation: CancellationSignal): ByteArray {
        val buffer=ByteArray(16384)
        val output=ByteArrayOutputStream()
        while (true) {
            cancellation.throwIfCanceled()
            val count=stream.read(buffer,0,minOf(buffer.size,MAX_BYTES-output.size()+1))
            if(count<0)break
            check(count>0)
            require(output.size()+count<=MAX_BYTES)
            output.write(buffer,0,count)
        }
        return output.toByteArray()
    }

    fun decodeBytes(bytes: ByteArray, cancellation: CancellationSignal=CancellationSignal()): String {
        require(bytes.size in 1..MAX_BYTES)
        val bounds=BitmapFactory.Options().apply { inJustDecodeBounds=true }
        BitmapFactory.decodeByteArray(bytes,0,bytes.size,bounds)
        require(bounds.outMimeType in listOf("image/png","image/jpeg","image/webp"))
        require(bounds.outWidth in 1..MAX_SOURCE && bounds.outHeight in 1..MAX_SOURCE)
        var sample=1
        while ((bounds.outWidth+sample-1)/sample>MAX_DECODE ||
            (bounds.outHeight+sample-1)/sample>MAX_DECODE) sample*=2
        cancellation.throwIfCanceled()
        val options=BitmapFactory.Options().apply {
            inSampleSize=sample
            inPreferredConfig=Bitmap.Config.ARGB_8888
        }
        val bitmap=BitmapFactory.decodeByteArray(bytes,0,bytes.size,options) ?: error("Invalid image")
        return try {
            cancellation.throwIfCanceled()
            decode(bitmap)
        } finally { bitmap.recycle() }
    }

    fun decode(bitmap: Bitmap): String {
        val width=bitmap.width
        val height=bitmap.height
        require(width in 1..MAX_DECODE && height in 1..MAX_DECODE && width.toLong()*height<=MAX_PIXELS)
        val pixels=IntArray(width*height)
        bitmap.getPixels(pixels,0,width,0,0,width,height)
        val image=BinaryBitmap(HybridBinarizer(RGBLuminanceSource(width,height,pixels)))
        val results=QRCodeMultiReader().decodeMultiple(image,mapOf(DecodeHintType.TRY_HARDER to true))
        require(results.size==1) { "Choose an image containing one QR code" }
        val code=results.single().text
        require(code.length in 1..MAX_CODE && code.all { it.code in 33..126 } &&
            (code.startsWith("IL1I.") || code.startsWith("IL1A."))) { "Invalid signed QR payload" }
        return code
    }
}
