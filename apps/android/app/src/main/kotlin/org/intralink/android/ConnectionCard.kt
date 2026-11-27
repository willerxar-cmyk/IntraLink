package org.intralink.android

import android.content.Context
import android.content.ClipboardManager
import android.content.ClipData
import android.net.ConnectivityManager
import java.net.Inet4Address

/** Public routing hint only. The Rust vault remains the pairing authority. */
internal data class ConnectionCard(val identity: String, val address: String?, val port: Int?) {
    fun encode(): String = if (address == null) "ILC1:BASE:$identity"
        else "ILC1:CLIENT:$identity:$address:$port"

    companion object {
        private val id = Regex("[0-9a-f]{64}")
        fun privateIp(value: String): Boolean {
            val octets = value.split('.')
            if (octets.size != 4) return false
            val n = octets.map { it.toIntOrNull() ?: return false }
            if (n.any { it !in 0..255 } || n.joinToString(".") != value) return false
            return n[0] == 10 || (n[0] == 172 && n[1] in 16..31) ||
                (n[0] == 192 && n[1] == 168)
        }

        fun parse(value: String): ConnectionCard? {
            if (value.length > 128 || value.any { it.code > 127 }) return null
            val parts = value.split(':')
            if (parts.size < 3 || parts[0] != "ILC1" || !id.matches(parts[2])) return null
            val card = when {
                parts[1] == "BASE" && parts.size == 3 -> ConnectionCard(parts[2], null, null)
                parts[1] == "CLIENT" && parts.size == 5 -> {
                    val port = parts[4].toIntOrNull() ?: return null
                    if (!privateIp(parts[3]) || port !in 1..65535) return null
                    ConnectionCard(parts[2], parts[3], port)
                }
                else -> return null
            }
            return card.takeIf { it.encode() == value }
        }

        /** Only prefill an unambiguous, active, private IPv4 route. */
        fun localAddress(context: Context): String? {
            val manager = context.getSystemService(ConnectivityManager::class.java)
            val network = manager.activeNetwork ?: return null
            return manager.getLinkProperties(network)?.linkAddresses
                ?.mapNotNull { (it.address as? Inet4Address)?.hostAddress }
                ?.filter(::privateIp)?.distinct()?.singleOrNull()
        }

        fun copy(context: Context, value: String) {
            context.getSystemService(ClipboardManager::class.java)
                .setPrimaryClip(ClipData.newPlainText("IntraLink connection card", value))
        }

        /** Only called by an explicit Paste action; no background clipboard read. */
        fun paste(context: Context): ConnectionCard? {
            val clip = context.getSystemService(ClipboardManager::class.java).primaryClip ?: return null
            if (clip.itemCount != 1) return null
            val text = clip.getItemAt(0).text ?: return null
            if (text.length > 128) return null
            return parse(text.toString().trim())
        }
    }
}
