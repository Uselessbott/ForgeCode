package dev.forgebox.data

data class ProjectSummary(
    val id: String,
    val name: String,
    val path: String,
    val createdAt: Long,
    val storageBytes: Long,
)

data class FileEntryModel(
    val name: String,
    val path: String,
    val isDir: Boolean,
    val size: Long,
)

data class FileStatModel(
    val path: String,
    val size: Long,
    val mtimeMillis: Long,
    val isTextHeuristic: Boolean,
)
