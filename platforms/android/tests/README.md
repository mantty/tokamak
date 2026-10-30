# Native request tests

With the Kotlin compiler and an Android SDK installed, run from the repository root:

```sh
ANDROID_JAR="$(ls "$ANDROID_HOME"/platforms/android-*/android.jar | sort -V | tail -1)"
kotlinc -classpath "$ANDROID_JAR" \
  plugins/core/android/src/main/kotlin/TokamakPlugin.kt \
  plugins/core/android/src/main/kotlin/TokamakResources.kt \
  plugins/core/android/src/main/kotlin/TokamakPluginRequests.kt \
  platforms/android/tests/plugin-requests.kt \
  -include-runtime -d /tmp/tokamak-plugin-requests.jar
# android.jar's org.json is stubs; the JVM runs the reference implementation.
curl -fsSL -o /tmp/json.jar https://repo1.maven.org/maven2/org/json/json/20250517/json-20250517.jar
java -cp "/tmp/tokamak-plugin-requests.jar:/tmp/json.jar:$ANDROID_JAR" com.tokamak.runtime.Plugin_requestsKt
```

The JVM tests exercise byte endpoints, executor selection, replies, cancellation and the shared
resource store.
They do not instantiate Android UI objects.
