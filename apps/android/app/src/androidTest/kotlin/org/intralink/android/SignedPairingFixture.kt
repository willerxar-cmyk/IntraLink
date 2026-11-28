package org.intralink.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.widget.CheckBox
import android.widget.EditText
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.nio.file.Files
import java.security.SecureRandom

/** Local real Android widgets and JNI; only a dedicated disposable emulator. */
internal object SignedPairingFixture {
    fun run(instrumentation: Instrumentation) {
        val results=Bundle()
        var result=Activity.RESULT_CANCELED
        val context=instrumentation.targetContext
        val path=File(context.filesDir,"profile.vault")
        val lock=File(context.filesDir,"profile.vault.lock")
        var owned=false
        var clipboardUsed=false
        var directory: File?=null
        var activity: Activity?=null
        val password=ByteArray(20).also(SecureRandom()::nextBytes)
            .joinToString("") { "%02x".format(it) }
        val secret=password.toByteArray(Charsets.UTF_8)
        try {
            check(Build.HARDWARE in listOf("ranchu","goldfish")) { "Disposable emulator required" }
            check(!path.exists() && !lock.exists()) { "Preserve existing profile" }
            val temp=Files.createTempDirectory(context.cacheDir.toPath(),"signed-enrollment-").toFile()
            directory=temp
            val basePath=File(temp,"base.vault").absolutePath
            fun ok(value:String)=JSONObject(value).also { check(it.optBoolean("ok")) { "Native operation failed: ${it.optString("error")}" } }
            val base=ok(NativeBridge.createProfile(basePath,0,"Fixture BASE",secret))
            owned=true
            val client=ok(NativeBridge.createProfile(path.absolutePath,1,"Fixture CLIENT",secret))
            val id=client.getString("public_id")
            val baseId=base.getString("public_id")
            fun <T> main(block:()->T):T {
                var outcome:Result<T>?=null
                instrumentation.runOnMainSync { outcome=runCatching(block) }
                return checkNotNull(outcome).getOrThrow()
            }
            fun launch():Activity = instrumentation.startActivitySync(Intent(context,SignedPairingActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra(SignedPairingActivity.ROLE,"CLIENT")
                .putExtra(SignedPairingActivity.IDENTITY,id))
            fun views(root:View):Sequence<View> = sequence {
                yield(root)
                if(root is ViewGroup) for(i in 0 until root.childCount) yieldAll(views(root.getChildAt(i)))
            }
            fun find(label:String):View?=views(checkNotNull(activity).findViewById<View>(android.R.id.content))
                .filter { it.isShown }.firstOrNull { it is TextView && (it.text.toString()==label || (it is EditText && it.hint?.toString()==label)) }
            fun await(label:String) {
                val deadline=SystemClock.elapsedRealtime()+15000
                while(SystemClock.elapsedRealtime()<deadline) {
                    if(main { find(label)!=null }) return
                    SystemClock.sleep(100)
                }
                error("Expected widget: $label")
            }
            fun click(label:String) {
                await(label)
                main { val view=checkNotNull(find(label)); check(view.isEnabled && view.performClick()) { "Action failed: $label" } }
                instrumentation.waitForIdleSync()
            }
            fun type(label:String,value:String) { await(label);main { (find(label) as EditText).setText(value) } }
            fun approve() {
                await("Review signed pairing")
                main {
                    val apply=checkNotNull(find(context.getString(R.string.apply)))
                    check(!apply.isEnabled) { "Apply enabled before verification" }
                    val verify=find(context.getString(R.string.signed_verify)) as CheckBox
                    check(!verify.isChecked)
                    verify.performClick()
                    check(apply.isEnabled)
                }
                type(context.getString(R.string.vault_passphrase),password)
                click(context.getString(R.string.apply))
            }
            activity=launch()
            click(context.getString(R.string.signed_create))
            type(context.getString(R.string.peer_identity),baseId)
            type(context.getString(R.string.local_ip),"192.168.10.2")
            type(context.getString(R.string.port),"45100")
            val original=path.readBytes()
            click(context.getString(R.string.review_pair))
            await(baseId)
            await(id)
            check(path.readBytes().contentEquals(original)) { "Review changed vault" }
            click(context.getString(R.string.cancel))
            check(path.readBytes().contentEquals(original))
            click(context.getString(R.string.signed_create))
            type(context.getString(R.string.peer_identity),baseId)
            type(context.getString(R.string.local_ip),"192.168.10.2")
            type(context.getString(R.string.port),"45100")
            click(context.getString(R.string.review_pair))
            approve()
            await(context.getString(R.string.signed_ready))
            val decodedFromImage=main {
                val qr=views(checkNotNull(activity).findViewById<View>(android.R.id.content)).filterIsInstance<SignedQrView>().single()
                val bitmap=Bitmap.createBitmap(512,512,Bitmap.Config.ARGB_8888)
                qr.layout(0,0,512,512)
                qr.draw(Canvas(bitmap))
                check(bitmap.getPixel(0,0)==Color.WHITE)
                check(bitmap.getPixel(511,511)==Color.WHITE)
                val pixels=IntArray(512*512)
                bitmap.getPixels(pixels,0,512,0,0,512,512)
                check(pixels.any { it==Color.BLACK } && pixels.all { it==Color.BLACK || it==Color.WHITE })
                val encoded=java.io.ByteArrayOutputStream()
                check(bitmap.compress(Bitmap.CompressFormat.PNG,100,encoded))
                bitmap.recycle()
                SignedQrImport.decodeBytes(encoded.toByteArray())
            }
            click(context.getString(R.string.signed_copy))
            clipboardUsed=true
            val offer=main {
                checkNotNull(context.getSystemService(android.content.ClipboardManager::class.java).primaryClip)
                    .getItemAt(0).text.toString()
            }
            check(offer.startsWith("IL1I.") && decodedFromImage==offer)
            check(runCatching { SignedQrImport.decodeBytes(ByteArray(8)) }.isFailure)
            check(ok(NativeBridge.openProfile(path.absolutePath,secret)).getInt("peer_count")==0)
            val accept=JSONObject().put("stage","accept").put("local_id",baseId).put("name","client").put("code",offer).toString()
            check(ok(NativeBridge.inspectEnrollment(accept)).getString("host_id")==id)
            val signed=ok(NativeBridge.commitEnrollment(basePath,accept,secret.copyOf()))
            check(signed.getInt("peer_count")==1)
            val reply=signed.getJSONObject("enrollment").getString("code")
            check(reply.startsWith("IL1A."))
            check(!JSONObject(NativeBridge.commitEnrollment(basePath,accept,secret.copyOf())).optBoolean("ok"))
            click(context.getString(R.string.signed_done))
            instrumentation.waitForIdleSync()
            activity=launch()
            click(context.getString(R.string.signed_confirm))
            type(context.getString(R.string.signed_code),offer)
            type(context.getString(R.string.peer_name),"base")
            click(context.getString(R.string.review_pair))
            await("Incorrect enrollment code type")
            type(context.getString(R.string.signed_code),reply)
            click(context.getString(R.string.review_pair))
            await(baseId)
            await(id)
            check(ok(NativeBridge.openProfile(path.absolutePath,secret)).getInt("peer_count")==0)
            approve()
            await(context.getString(R.string.signed_complete))
            val stored=ok(NativeBridge.openProfile(path.absolutePath,secret))
            check(stored.getInt("peer_count")==1)
            check(stored.getJSONArray("peers").getJSONObject(0).getString("identity")==baseId)
            val confirm=JSONObject().put("stage","confirm").put("local_id",id).put("name","base").put("code",reply).toString()
            check(!JSONObject(NativeBridge.commitEnrollment(path.absolutePath,confirm,secret.copyOf())).optBoolean("ok"))
            results.putString("stream","\nSIGNED_PAIRING_PASS review_read_only=true cancel=true qr_native=true qr_image_roundtrip=true issue=true accept=true wrong_kind_rejected=true confirm=true replay_rejected=true\n")
            result=Activity.RESULT_OK
        } catch(error:Exception) {
            results.putString("stream","\nSigned pairing FAILED: ${error.javaClass.simpleName}: ${error.message}\n")
        } finally {
            secret.fill(0)
            instrumentation.runOnMainSync {
                if(clipboardUsed) context.getSystemService(android.content.ClipboardManager::class.java).clearPrimaryClip()
                activity?.finish()
            }
            instrumentation.waitForIdleSync()
            if(owned) { path.delete();lock.delete() }
            directory?.deleteRecursively()
        }
        instrumentation.finish(result,results)
    }
}
