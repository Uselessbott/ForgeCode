package dev.forgebox.data

import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.zip.ZipInputStream

object GithubRepoImporter {
    data class Spec(val owner: String, val repo: String, val ref: String?)

    fun parse(url: String): Spec {
        val clean = url.trim().removeSuffix(".git").removeSuffix("/")
        val m = Regex("""https?://github\.com/([^/]+)/([^/]+)(?:/(?:tree|blob)/([^/]+).*)?""", RegexOption.IGNORE_CASE)
            .matchEntire(clean) ?: throw IllegalArgumentException("Unsupported GitHub URL")
        return Spec(m.groupValues[1], m.groupValues[2], m.groupValues[3].ifBlank { null })
    }

    fun defaultName(spec: Spec): String = spec.repo

    fun downloadZip(spec: Spec, destZip: File, maxBytes: Long = 64L * 1024 * 1024) {
        val candidates = mutableListOf<String>()
        spec.ref?.let { candidates += "https://codeload.github.com/${spec.owner}/${spec.repo}/zip/refs/heads/$it" }
        spec.ref?.let { candidates += "https://codeload.github.com/${spec.owner}/${spec.repo}/zip/refs/tags/$it" }
        candidates += "https://codeload.github.com/${spec.owner}/${spec.repo}/zip/refs/heads/main"
        candidates += "https://codeload.github.com/${spec.owner}/${spec.repo}/zip/refs/heads/master"
        var lastErr: Exception? = null
        for (u in candidates.distinct()) {
            try {
                download(u, destZip, maxBytes)
                return
            } catch (e: Exception) {
                lastErr = e
            }
        }
        throw lastErr ?: IllegalStateException("Download failed")
    }

    private fun download(url: String, dest: File, maxBytes: Long) {
        val conn = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15000
            readTimeout = 30000
            instanceFollowRedirects = true
            setRequestProperty("User-Agent", "ForgeBox")
        }
        conn.connect()
        val code = conn.responseCode
        if (code !in 200..299) {
            conn.disconnect()
            throw IllegalStateException("HTTP $code for $url")
        }
        dest.parentFile?.mkdirs()
        conn.inputStream.use { input ->
            FileOutputStream(dest).use { out ->
                val buf = ByteArray(8192)
                var total = 0L
                while (true) {
                    val n = input.read(buf)
                    if (n <= 0) break
                    total += n
                    if (total > maxBytes) throw IllegalStateException("Repo ZIP too large")
                    out.write(buf, 0, n)
                }
            }
        }
        conn.disconnect()
    }

    fun extractZip(zipFile: File, outDir: File) {
        outDir.mkdirs()
        ZipInputStream(zipFile.inputStream().buffered()).use { zis ->
            while (true) {
                val entry = zis.nextEntry ?: break
                val name = entry.name.replace('\\', '/')
                if (name.isBlank() || name.contains("..")) { zis.closeEntry(); continue }
                val stripped = name.substringAfter('/', name)
                if (stripped.isBlank()) { zis.closeEntry(); continue }
                val target = File(outDir, stripped).canonicalFile
                val base = outDir.canonicalFile
                if (!target.path.startsWith(base.path)) { zis.closeEntry(); continue }
                if (entry.isDirectory) {
                    target.mkdirs()
                } else {
                    target.parentFile?.mkdirs()
                    FileOutputStream(target).use { out -> zis.copyTo(out) }
                }
                zis.closeEntry()
            }
        }
    }
}
