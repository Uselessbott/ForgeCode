package dev.forgebox.app.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class EditorDiskConflictTest {
    @Test
    fun cleanTabReloadsOnDiskChange() {
        val decision = EditorViewModel.diskChangeDecision(dirty = false, knownMtimeMillis = 100, diskMtimeMillis = 200)
        assertEquals(EditorViewModel.DiskChangeDecision.RELOAD, decision)
    }

    @Test
    fun dirtyTabConflictsOnDiskChange() {
        val decision = EditorViewModel.diskChangeDecision(dirty = true, knownMtimeMillis = 100, diskMtimeMillis = 200)
        assertEquals(EditorViewModel.DiskChangeDecision.CONFLICT, decision)
    }

    @Test
    fun sameMtimeDoesNothing() {
        val decision = EditorViewModel.diskChangeDecision(dirty = true, knownMtimeMillis = 100, diskMtimeMillis = 100)
        assertEquals(EditorViewModel.DiskChangeDecision.NONE, decision)
    }
}
