//! Generation from small SDK descriptions, through each emitter.

use serde_json::{Value, json};

use super::model::Module;
use super::{android, apple, kotlin, prune, swift, typescript};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Kit's iOS symbol graph: the macOS SDK lacks `KTWidget.measure()`, the
/// `async` import of `measure(completion:)`.
const KIT: &str = include_str!("fixtures/Kit.symbols.json");

fn kit_module() -> TestResult<Module> {
    kit_module_with(|_| {})
}

/// Kit's module from its graphs after `edit` changes their symbols.
fn kit_module_with(edit: impl Fn(&mut Vec<Value>)) -> TestResult<Module> {
    let mut ios: Value = serde_json::from_str(KIT)?;
    let mut macos = ios.clone();
    if let Some(symbols) = macos["symbols"].as_array_mut() {
        symbols.retain(|symbol| symbol["pathComponents"] != json!(["KTWidget", "measure()"]));
    }
    for graph in [&mut ios, &mut macos] {
        if let Some(symbols) = graph["symbols"].as_array_mut() {
            edit(symbols);
        }
    }
    let graphs = vec![
        ("iOS", "Kit".to_owned(), serde_json::from_value(ios)?),
        ("macOS", "Kit".to_owned(), serde_json::from_value(macos)?),
    ];
    let mut module = apple::module(
        "kit",
        &["Kit".to_owned()],
        "the test SDKs: Kit".to_owned(),
        graphs,
    );
    prune(&mut module);
    Ok(module)
}

#[test]
fn apple_bindings_mirror_classes_values_protocols_and_streams() -> TestResult {
    let module = kit_module()?;
    let ts = typescript::emit(&module);
    for expected in [
        "const bindings = new Bindings(\"kit/apple\", () => classes);",
        "export type KTDistance = number;",
        "export type KTMode = \"fast\" | \"slow\";",
        "  checkError(value: number): Promise<boolean> {\n    return bindings.call(this, \"KTWidget.check(_:error:)\", [value]);",
        "export interface KTPoint {\n  x: number;\n  y: number;\n}",
        "export class KTWidget extends NativeObject {",
        "  static init(ctx: TokamakContext): Promise<KTWidget> {\n    return bindings.call(ctx, \"KTWidget.init()\", []);",
        "  setTitle(value: string): Promise<void> {\n    return bindings.call(this, \"KTWidget.title=\", [value]);",
        "  startWithMode(mode: KTMode): Promise<void> {",
        "  /** iOS 13.0+ */\n  measure(options?: { signal?: AbortSignal }): Promise<KTDistance> {\n    return bindings.call(this, \"KTWidget.measure()\", [], options?.signal);",
        "  configureOptions(options_: KTMode, options?: { signal?: AbortSignal }): Promise<void> {\n    return bindings.call(this, \"KTWidget.configure(options:)\", [options_], options?.signal);",
        "  readings(): ReadableStream<KTPoint> {\n    return bindings.stream(this, \"KTWidget.readings\", []);",
        "  widgetDidMove?(widget: KTWidget, point: KTPoint): void | Promise<void>;",
        "return bindings.implement(ctx, KTWidgetDelegate, \"KTWidgetDelegate\", [\"widgetDidMove\"], handlers);",
        "  \"KTWidget\": KTWidget,",
        "  \"KTWidgetDelegate\": KTWidgetDelegate,",
    ] {
        assert!(ts.contains(expected), "missing {expected:?} in\n{ts}");
    }
    assert!(
        !ts.contains("Readings"),
        "async sequences are streams, not types:\n{ts}"
    );
    Ok(())
}

#[test]
fn apple_native_plugin_calls_guards_and_implements() -> TestResult {
    let swift = swift::emit(&kit_module()?, "kit");
    for expected in [
        "final class TokamakBindings_kit: TokamakPlugin {",
        "  let id = \"kit/apple\"",
        "      return b.run { let value = KTWidget(); return b.encode(object: value) }",
        "      return b.run { try b.target(KTWidget.self).title = b.decode(b[0], as: String.self); return nil }",
        "      return b.run { try b.target(KTWidget.self).start(with: TokamakBindings_kit.decode(b[0], b, as: KTMode.self)); return nil }",
        "    #if os(iOS)\n    case \"KTWidget.measure()\":\n      guard #available(iOS 13.0, *) else { return b.run { throw TokamakBinding.unavailable } }\n      return b.task { let value = try await b.target(KTWidget.self).measure(); return b.encode(value) }\n    #endif",
        "      return b.stream({ try b.target(KTWidget.self).readings }, encode: { TokamakBindings_kit.encode($0, b) })",
        "      return b.implement(KTWidgetDelegateImplementation.self)",
        "    if #available(macOS 12.0, *), value == .fast { return \"fast\" }",
        "    case \"fast\":\n      guard #available(macOS 12.0, *) else { throw TokamakBinding.unavailable }\n      return .fast",
        "      return b.run { var error: NSError?; let value = try b.target(KTWidget.self).check(b.decode(b[0], as: Int.self), error: &error); if let error { throw error }; return b.encode(value) }",
        "    try KTPoint(\n      x: b.decode(b.field(value, \"x\"), as: Double.self),",
        "        NSSelectorFromString(\"widget:didMove:\"): \"widgetDidMove\",",
        "    func widget(_ widget: KTWidget, didMove point: KTPoint) {\n      call(\"widgetDidMove\") { b in [b.encode(object: widget), TokamakBindings_kit.encode(point, b)] }",
    ] {
        assert!(swift.contains(expected), "missing {expected:?} in\n{swift}");
    }
    Ok(())
}

#[test]
fn apple_bindings_do_not_depend_on_the_order_of_extracted_symbols() -> TestResult {
    let forward = kit_module()?;
    let reversed = kit_module_with(|symbols| symbols.reverse())?;
    assert_eq!(swift::emit(&forward, "kit"), swift::emit(&reversed, "kit"));
    assert_eq!(typescript::emit(&forward), typescript::emit(&reversed));
    Ok(())
}

/// A stub source and API levels in the Android SDK's formats.
const WIDGET: &str = include_str!("fixtures/Widget.java");

const LEVELS: &str = include_str!("fixtures/api-versions.xml");

fn widget_module() -> TestResult<Module> {
    let levels = android::levels(LEVELS.as_bytes())?;
    let files = vec![("android/kit/Widget.java".to_owned(), WIDGET.to_owned())];
    let mut module = android::module(
        "kit",
        "the test SDK: android.kit".to_owned(),
        files,
        &levels,
    )?;
    prune(&mut module);
    Ok(module)
}

#[test]
fn android_bindings_select_overloads_and_keep_constants() -> TestResult {
    let module = widget_module()?;
    let ts = typescript::emit(&module);
    for expected in [
        "  /** Android API 26+ */\n  static readonly MODE_FAST = \"fast\";",
        "  static readonly LIMIT = 10;",
        "  static new(ctx: TokamakContext): Promise<Widget> {",
        "  /** Android API 24+ */\n  start(speed: number): Promise<void>;",
        "  start(mode: string): Promise<void>;",
        "  start(...args: unknown[]): Promise<unknown> {\n    return bindings.call(this, bindings.overload(args, [",
        "      [\"Widget#start(Ljava/lang/String;)V\", [[\"string\"]]],",
        "export class Widget$Builder extends NativeObject {",
        "  onStopped?(): void | Promise<void>;",
        "  \"android.kit.Widget$Builder\": Widget$Builder,",
    ] {
        assert!(ts.contains(expected), "missing {expected:?} in\n{ts}");
    }
    let stops = ts.matches("Widget#stop(").count();
    assert_eq!(
        stops, 1,
        "TypeScript cannot tell stop(int) from stop(long):\n{ts}"
    );
    assert!(
        module
            .skipped
            .iter()
            .any(|skipped| skipped.symbol == "Widget#stop(J)V")
    );
    assert!(
        module
            .skipped
            .iter()
            .any(|skipped| skipped.symbol == "android.kit.Widget.Widget")
    );
    Ok(())
}

#[test]
fn android_native_plugin_dispatches_by_type_and_implements_listeners() -> TestResult {
    let kotlin = kotlin::emit(&widget_module()?, "kit");
    for expected in [
        "package com.tokamak.bindings.kit",
        "    override val id = \"kit/android\"",
        "            \"Widget.Builder\" -> callWidget_Builder(method, b)",
        "            \"Widget#start(I)V\" -> b.run { if (Build.VERSION.SDK_INT < 24) throw TokamakBinding.unavailable; b.target(android.kit.Widget::class.java).start(b.int(b[0])); null }",
        "            \"Widget#apply(Landroid/kit/Widget\\$Builder;)V\" -> b.run {",
        "            \"Widget#<init>()V\" -> b.run { if (Build.VERSION.SDK_INT < 21) throw TokamakBinding.unavailable; b.encode(android.kit.Widget()) }",
        "            \"Widget.Listener\" -> {\n                if (Build.VERSION.SDK_INT < 29) throw TokamakBinding.unavailable\n                b.implement { Widget_ListenerImplementation(it) }",
        "    @androidx.annotation.RequiresApi(29)\n    private class Widget_ListenerImplementation(private val binding: TokamakBinding) : android.kit.Widget.Listener {",
        "        override fun onMoved(x: Int) {\n            binding.call(\"onMoved\") { b -> listOf(b.encode(x)) }",
        "        override fun onStopped() {\n            if (!binding.call(\"onStopped\") { b -> listOf() }) super.onStopped()",
    ] {
        assert!(
            kotlin.contains(expected),
            "missing {expected:?} in\n{kotlin}"
        );
    }
    Ok(())
}
