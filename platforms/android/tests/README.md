# Native request tests

With the Kotlin compiler and an Android SDK installed, run from the repository root:

```sh
ANDROID_JAR="$ANDROID_HOME/platforms/android-35/android.jar"
kotlinc -classpath "$ANDROID_JAR" \
  platforms/android/source/plugin/TokamakPlugin.kt \
  platforms/android/source/app/TokamakPluginRequests.kt \
  platforms/android/tests/plugin-requests.kt \
  -include-runtime -d /tmp/tokamak-plugin-requests.jar
java -cp "/tmp/tokamak-plugin-requests.jar:$ANDROID_JAR" com.tokamak.runtime.Plugin_requestsKt
```

The JVM tests exercise byte endpoints, executor selection, replies and cancellation.
They do not instantiate Android UI objects.
