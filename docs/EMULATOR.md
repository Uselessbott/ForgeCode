# Emulator

## AVD
- Name: `ForgeBox_API31`
- Path: `C:\Users\Lahari\.android\avd\ForgeBox_API31.avd`
- Configured target: `android-31`
- ABI: `x86_64`
- Device profile: `pixel_5` (Google)
- Play Store: no

## Tools
- SDK: `C:\Users\Lahari\AppData\Local\Android\Sdk`
- adb: `C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe`
- emulator: `C:\Users\Lahari\AppData\Local\Android\Sdk\emulator\emulator.exe`

## Available device (Option B - no supported emulator)
- Serial: `emulator-5554`
- adb-reported: `ro.build.version.release=9`, `ro.build.version.sdk=28`, `ro.product.cpu.abi=x86_64`, `ro.product.model=SM-N9810`
- **Only API 28 emulator available (below app minSdk 31).** APK install succeeds, but launch crashes on this device:
  - `FATAL EXCEPTION: main` in `dev.forgebox.app`
  - `java.lang.NoSuchMethodError: No static method isEnabled()Z in class Landroid/os/Trace; or its super classes`
  - Stack: `androidx.tracing.Trace.isEnabled` / `androidx.startup.InitializationProvider.onCreate`
- App `minSdk=31`, `targetSdk=36`, `compileSdk=37`. This device is not a supported verification target.
- **Decision**: Option B. Proceed with Gradle + cargo gates only. M8+ docs must state: "no emulator runtime verification on a supported device; only API 28 emulator available" and mark runtime-only claims as not runtime-verified with this reason.

## Install / launch commands (for supported API 31+ device)
```powershell
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" devices -l
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" -s <serial> install -r app\build\outputs\apk\debug\app-debug.apk
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" -s <serial> shell am start -W -n dev.forgebox.app/.MainActivity
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" -s <serial> logcat -d
```
