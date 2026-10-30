plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.tokamak.plugins.notifications"
    compileSdk = 35
    defaultConfig {
        minSdk = 31
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    implementation(project(":tokamak-plugin"))
    implementation("com.google.firebase:firebase-messaging:25.1.3")
}
