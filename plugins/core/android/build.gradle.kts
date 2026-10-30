// The plugin API, which plugins' modules depend on as project(":tokamak-plugin"). Its minimum SDK
// is the pack's. Coroutines are the version androidx.lifecycle brings, so plugins' coroutines add
// no code.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.tokamak.runtime.plugin"
    compileSdk = 35
    defaultConfig {
        minSdk = 31
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    api("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
}
