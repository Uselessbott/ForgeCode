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

## Observed running device during M7 closeout
- Serial: `emulator-5554`
- `ro.build.version.release=9`, `ro.build.version.sdk=28` (adb-reported; mismatches AVD config name)
- `ro.product.cpu.abi=x86_64`
- `ro.product.model=SM-N9810`

## Install / launch commands
```powershell
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" devices -l
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" -s emulator-5554 install -r app\build\outputs\apk\debug\app-debug.apk
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" -s emulator-5554 shell am start -W -n dev.forgebox.app/.MainActivity
& "C:\Users\Lahari\AppData\Local\Android\Sdk\platform-tools\adb.exe" -s emulator-5554 logcat -d
```

## M7 closeout result
- Install: **Success** (`adb install -r`).
- Launch: **FAILED on emulator-5554 (API 28)**. `am start` returned and Launcher remained resumed; logcat shows fatal crash:
  - `FATAL EXCEPTION: main` in `dev.forgebox.app`
  - `java.lang.NoSuchMethodError: No static method isEnabled()Z in class Landroid/os/Trace; or its super classes`
  - Stack includes `androidx.tracing.Trace.isEnabled` / `androidx.startup.InitializationProvider.onCreate`
- App `minSdk=31` / `targetSdk=36` / `compileSdk=37`; API 28 device is below minSdk and not a supported verification target. Emulator verification of M7 runtime behavior is **blocked on an API 31+ emulator/device in this environment**.
