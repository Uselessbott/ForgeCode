package dev.forgebox.app

import android.app.Application
import dev.forgebox.core.ForgeNative

class ForgeBoxApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        // Loading here surfaces a broken native build at startup, not mid-session.
        ForgeNative.version
    }
}
