package dev.forgebox.core

import dev.forgebox.proto.ForgeProto

/** Minimal transport seam so ViewModels/repositories can be unit-tested with fakes. */
interface ForgeBoxCore {
    fun call(request: ForgeProto.Request): ForgeProto.Response
}
