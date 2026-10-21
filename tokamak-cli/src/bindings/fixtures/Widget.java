package android.kit;
public class Widget {
public Widget() { throw new RuntimeException("Stub!"); }
public Widget(android.content.Context context) { throw new RuntimeException("Stub!"); }
public void start(int speed) { throw new RuntimeException("Stub!"); }
public void start(@android.annotation.NonNull java.lang.String mode) { throw new RuntimeException("Stub!"); }
public void stop(int code) { throw new RuntimeException("Stub!"); }
public void stop(long code) { throw new RuntimeException("Stub!"); }
public void apply(@android.annotation.NonNull android.kit.Widget.Builder builder) { throw new RuntimeException("Stub!"); }
public static final java.lang.String MODE_FAST = "fast";
public static final long LIMIT = 10L;
public static class Builder {
public Builder() { throw new RuntimeException("Stub!"); }
@android.annotation.NonNull
public android.kit.Widget build() { throw new RuntimeException("Stub!"); }
}
public static interface Listener {
public void onMoved(int x);
public default void onStopped() { throw new RuntimeException("Stub!"); }
}
}
