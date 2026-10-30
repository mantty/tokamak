// The plugin's module on its own, with the plugin API it depends on, for an IDE and Gradle. Apps
// build it with their own settings and the pack's versions of these build tools.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
    plugins {
        id("com.android.library") version "8.7.3"
        id("org.jetbrains.kotlin.android") version "2.1.0"
    }
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "notifications"
include(":tokamak-plugin")
project(":tokamak-plugin").projectDir = file("../node_modules/@tokamakdev/plugin/android")
