plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

val rustCoreDir = rootProject.file("rust/forgebox_core")
val jniLibsDir = file("src/main/jniLibs")

val buildRustCore = tasks.register<Exec>("buildRustCore") {
    description = "Cross-compile the ForgeBox Rust core for supported Android ABIs."
    group = "forgebox"
    workingDir = rustCoreDir
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "-t", "x86_64",
        "-o", jniLibsDir.absolutePath,
        "build", "--release", "--link-builtins",
    )
}

tasks.named("preBuild") {
    dependsOn(buildRustCore)
}

android {
    namespace = "dev.forgebox.app"
    // Target stays 36 for Play; newer AndroidX requires a newer compile SDK.
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.forgebox.app"
        minSdk = 31
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-m2"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        jniLibs {
            // Required for the W^X-safe executable helper binaries planned in M12.
            // AGP 9 rejects the legacy manifest attribute and requires this flag.
            useLegacyPackaging = true
        }
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.05.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.core:core-ktx:1.19.1")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.activity:activity-ktx:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.11.0")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    debugImplementation("androidx.compose.ui:ui-tooling")

    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
