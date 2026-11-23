package dev.forgebox.data

data class GitFileStatusModel(val path: String, val status: String, val isBinary: Boolean)
data class GitStatusModel(val staged: List<GitFileStatusModel>, val unstaged: List<GitFileStatusModel>, val untracked: List<GitFileStatusModel>, val branch: String)
data class GitCommitModel(val commitId: String, val author: String, val email: String, val message: String, val timestamp: Long)
data class GitBranchModel(val name: String, val isCurrent: Boolean)
