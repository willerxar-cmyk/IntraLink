package org.intralink.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import org.json.JSONObject
import java.io.File
import java.nio.file.Files
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicReference

/** Explicit instrumentation only: a temporary vault and real app canvas. */
internal object InputLanFixture {
    private fun find(view: View): InputCanvas? {
        if (view is InputCanvas) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) {
            find(view.getChildAt(i))?.let { return it }
        }
        return null
    }

    fun run(instrumentation: Instrumentation, args: Bundle) {
        val result = Bundle()
        val secret = ByteArray(32).also(SecureRandom()::nextBytes)
        var directory: File? = null
        var activity: Activity? = null
        var reservation = 0L
        var worker: Thread? = null
        var code = Activity.RESULT_CANCELED
        try {
            val context=instrumentation.targetContext
            val base=requireNotNull(args.getString("peerId"))
            val host=requireNotNull(args.getString("host"))
            val port=requireNotNull(args.getString("port")).toInt()
            check(ConnectionCard.privateIp(host) && port in 1024..65535)
            val temp=Files.createTempDirectory(context.cacheDir.toPath(),"intralink-input-fixture-").toFile()
            directory=temp
            val path=File(temp,"profile.vault").absolutePath
            fun success(value: String) = JSONObject(value).also { check(it.optBoolean("ok")) }
            val profile=success(NativeBridge.createProfile(path,1,"Input test",secret))
            val peer=JSONObject().put("name","test-base").put("peer_id",base).put("direction","inbound")
            success(NativeBridge.pairDevice(path,peer.toString(),secret))
            check(NativeBridge.initializeNetwork(context.applicationContext))
            val screen=instrumentation.startActivitySync(Intent(context,InputPreviewActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra(ScreenShareActivity.EXTRA_NAME,"test-base")
                .putExtra(ScreenShareActivity.EXTRA_IDENTITY,base))
            activity=screen
            var canvas: InputCanvas?=null
            instrumentation.runOnMainSync {
                canvas=find(screen.window.decorView)
            }
            val target=requireNotNull(canvas)
            val ready=SystemClock.elapsedRealtime()+5000
            var visible=false
            while (!visible && SystemClock.elapsedRealtime()<ready) {
                instrumentation.runOnMainSync { visible=target.isShown && target.hasWindowFocus() && target.width>0 && target.height>0 }
                if (!visible) Thread.sleep(50)
            }
            check(visible) { "App canvas not visible" }
            instrumentation.runOnMainSync { target.begin() }
            val id=NativeBridge.reserveProbe()
            check(id>0)
            reservation=id
            val request=JSONObject().put("name","test-base").put("peer_id",base)
                .put("host",host).put("port",port).toString()
            val response=AtomicReference<String>()
            val password=secret.copyOf()
            worker=Thread({
                try { response.set(NativeBridge.runInput(path,request,id,password,target)) }
                finally { password.fill(0) }
            },"IntraLink-input-fixture")
            instrumentation.sendStatus(1,Bundle().apply {
                putString("stream","\nINPUT_ANDROID_ID ${profile.getString("public_id")}\n")
            })
            worker.start()
            val end=SystemClock.elapsedRealtime()+20000
            var cursorObserved=false
            while (worker.isAlive && SystemClock.elapsedRealtime()<end) {
                instrumentation.runOnMainSync {
                    val state=target.snapshot()
                    if (!cursorObserved && state.active && state.x==32768 && state.y==32768 && state.moves>0) {
                        val bitmap=Bitmap.createBitmap(target.width,target.height,Bitmap.Config.ARGB_8888)
                        try {
                            target.draw(Canvas(bitmap))
                            val x=(state.x.toFloat()/65535f*(target.width-1)).toInt()
                            val y=(state.y.toFloat()/65535f*(target.height-1)).toInt()
                            val extent=(26*target.resources.displayMetrics.density).toInt()
                            var bright=0
                            for (dy in 0..extent) for (dx in 0..extent) {
                                if (x+dx<bitmap.width && y+dy<bitmap.height && bitmap.getPixel(x+dx,y+dy)==Color.WHITE) bright++
                            }
                            cursorObserved=bright>12
                        } finally { bitmap.recycle() }
                    }
                }
                Thread.sleep(40)
            }
            check(!worker.isAlive) { "Input fixture deadline" }
            val ack=success(response.get())
            var state: InputCanvas.Snapshot?=null
            instrumentation.runOnMainSync { state=target.snapshot(); target.stop() }
            val observed=requireNotNull(state)
            check(cursorObserved && observed.moves==2 && observed.clicks==1 && observed.scroll==3 &&
                observed.x==48000 && observed.y==41000 && observed.text=="Ab" &&
                !observed.active && !observed.held && observed.sequence==ack.getInt("input_records")) {
                "Input canvas verification failed: cursor=$cursorObserved, moves=${observed.moves}, clicks=${observed.clicks}, scroll=${observed.scroll}, sequence=${observed.sequence}"
            }
            success(NativeBridge.revokeDevice(path,"test-base",secret))
            val next=NativeBridge.reserveProbe()
            check(next>id)
            NativeBridge.cancelProbe(next)
            result.putString("stream","\nINPUT_ANDROID_PASSED records=${observed.sequence} moves=2 clicks=1 scroll=3 cursor=true text_verified=true released=true revoked=true\n")
            code=Activity.RESULT_OK
        } catch (error: Exception) {
            result.putString("stream","\nINPUT_ANDROID_FAILED: ${error.javaClass.simpleName}: ${error.message}\n")
        } catch (_: LinkageError) {
            result.putString("stream","\nINPUT_ANDROID_FAILED: native boundary unavailable\n")
        } finally {
            if (reservation>0) NativeBridge.cancelProbe(reservation)
            if (activity!=null) instrumentation.runOnMainSync { activity?.finish() }
            worker?.join(3000)
            secret.fill(0)
            if (worker?.isAlive!=true) directory?.deleteRecursively()
        }
        instrumentation.finish(code,result)
    }
}
