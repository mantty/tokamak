//! The JNI seam between the Kotlin shell and the tokamak runtime.
//!
//! The shell owns the Activity, `WebView`, and lifecycle. This crate only
//! moves values across the boundary; every decision belongs to the runtime.

use std::ffi::{CString, c_char, c_int};
use std::time::Duration;

use crate::{Challenge, Decision, Event, Runtime, bridge};
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JObjectArray, JString};
use jni::sys::{jboolean, jint, jlong};

const LOG_TAG: &str = "tokamak";
const LOG_INFO: c_int = 4;
const LOG_ERROR: c_int = 6;
const FAILURE: &str = "java/lang/IllegalStateException";

unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Start the runtime, which delivers `start` with `foreground`, and return an
/// opaque handle, or throw on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeStart(
    mut env: JNIEnv,
    _: JClass,
    packaged_dir: JString,
    state_dir: JString,
    storage_dir: JString,
    host: JString,
    foreground: jboolean,
) -> jlong {
    let result = start(
        &mut env,
        &packaged_dir,
        &state_dir,
        &storage_dir,
        &host,
        foreground != 0,
    );
    into_handle(&mut env, result, "runtime startup failed")
}

/// Start the development runtime, which delivers `start` with `foreground`,
/// and return an opaque handle, or throw on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeStartDevelopment(
    mut env: JNIEnv,
    _: JClass,
    state_dir: JString,
    host: JString,
    endpoint: JString,
    session_token: JString,
    foreground: jboolean,
) -> jlong {
    let result = start_development(
        &mut env,
        &state_dir,
        &host,
        &endpoint,
        &session_token,
        foreground != 0,
    );
    into_handle(&mut env, result, "development runtime startup failed")
}

/// Return the loopback port the gateway bound.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativePort(
    _: JNIEnv,
    _: JClass,
    handle: jlong,
) -> jint {
    runtime(handle).map_or(0, |runtime| jint::from(runtime.port()))
}

/// Wait for the runtime's loopback gateway and return its current port.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeRestoreGateway(
    mut env: JNIEnv,
    _: JClass,
    handle: jlong,
) -> jint {
    let Some(runtime) = runtime(handle) else {
        let _ = env.throw_new(FAILURE, "tokamak runtime is unavailable");
        return 0;
    };
    match runtime.restore_gateway() {
        Ok(port) => jint::from(port),
        Err(error) => {
            let _ = env.throw_new(FAILURE, error.to_string());
            0
        }
    }
}

/// Deliver the event `name`, whose JSON is `event`, to the Worker's
/// listeners, blocking for up to `timeout_millis`. Returns the reply's JSON;
/// throws when the event fails.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeEmit<'local>(
    mut env: JNIEnv<'local>,
    _: JClass,
    handle: jlong,
    name: JString,
    event: JString,
    timeout_millis: jlong,
) -> JString<'local> {
    let timeout = Duration::from_millis(u64::try_from(timeout_millis).unwrap_or(0));
    let result = emit(&mut env, handle, &name, &event, timeout)
        .and_then(|reply| env.new_string(reply).map_err(|error| error.to_string()));
    result.unwrap_or_else(|message| {
        let _ = env.throw_new(FAILURE, message);
        JString::from(JObject::null())
    })
}

/// Return the authority a server certificate for `host` must chain to, or
/// null when tokamak does not vouch for the host.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeServerAuthority<'local>(
    mut env: JNIEnv<'local>,
    _: JClass,
    handle: jlong,
    host: JString,
) -> JByteArray<'local> {
    let Some(runtime) = runtime(handle) else {
        return null_array();
    };
    let Ok(host) = text(&mut env, &host) else {
        return null_array();
    };
    match runtime
        .certificates()
        .decide(&Challenge::ServerTrust { host: &host })
    {
        Decision::TrustAuthority(authority) => env
            .byte_array_from_slice(&authority)
            .unwrap_or_else(|_| null_array()),
        _ => null_array(),
    }
}

/// Return `[certificate, private key]` in DER for `host`, or null when tokamak
/// cannot authenticate the connection.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeClientIdentity<'local>(
    mut env: JNIEnv<'local>,
    _: JClass,
    handle: jlong,
    host: JString,
    previous_failures: jint,
) -> JObjectArray<'local> {
    let Some(runtime) = runtime(handle) else {
        return null_object_array();
    };
    let Ok(host) = text(&mut env, &host) else {
        return null_object_array();
    };
    let decision = runtime
        .certificates()
        .decide(&Challenge::ClientCertificate {
            host: &host,
            previous_failures: usize::try_from(previous_failures).unwrap_or(usize::MAX),
        });
    let Decision::PresentIdentity {
        certificate,
        private_key,
    } = decision
    else {
        return null_object_array();
    };
    identity(&mut env, &certificate, &private_key).unwrap_or_else(|_| null_object_array())
}

fn start(
    env: &mut JNIEnv,
    packaged_dir: &JString,
    state_dir: &JString,
    storage_dir: &JString,
    host: &JString,
    foreground: bool,
) -> Result<Runtime, String> {
    bridge::start(
        &text(env, packaged_dir)?,
        &text(env, state_dir)?,
        &text(env, storage_dir)?,
        &text(env, host)?,
        foreground,
        report,
    )
}

fn start_development(
    env: &mut JNIEnv,
    state_dir: &JString,
    host: &JString,
    endpoint: &JString,
    session_token: &JString,
    foreground: bool,
) -> Result<Runtime, String> {
    bridge::start_development(
        &text(env, state_dir)?,
        &text(env, host)?,
        &text(env, endpoint)?,
        &text(env, session_token)?,
        foreground,
        report,
    )
}

fn emit(
    env: &mut JNIEnv,
    handle: jlong,
    name: &JString,
    event: &JString,
    timeout: Duration,
) -> Result<String, String> {
    let runtime = runtime(handle).ok_or("tokamak runtime is unavailable")?;
    let name = text(env, name)?;
    let event = text(env, event)?;
    runtime
        .emit(&name, &event, timeout)
        .map_err(|error| error.to_string())
}

fn report(event: &Event) {
    let priority = match event {
        Event::Failed { .. } | Event::RequestFailed { .. } => LOG_ERROR,
        _ => LOG_INFO,
    };
    log(priority, &event.to_string());
}

fn into_handle(env: &mut JNIEnv, result: Result<Runtime, String>, failure: &str) -> jlong {
    match result {
        Ok(runtime) => Box::into_raw(Box::new(runtime)) as jlong,
        Err(message) => {
            log(LOG_ERROR, &format!("{failure}: {message}"));
            let _ = env.throw_new(FAILURE, message);
            0
        }
    }
}

fn runtime<'handle>(handle: jlong) -> Option<&'handle Runtime> {
    if handle == 0 {
        return None;
    }
    // SAFETY: `handle` came from `Box::into_raw` in `nativeStart` and the
    // runtime lives for the rest of the process.
    Some(unsafe { &*(handle as *const Runtime) })
}

fn text(env: &mut JNIEnv, value: &JString) -> Result<String, String> {
    env.get_string(value)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

fn identity<'local>(
    env: &mut JNIEnv<'local>,
    certificate: &[u8],
    private_key: &[u8],
) -> Result<JObjectArray<'local>, jni::errors::Error> {
    let class = env.find_class("[B")?;
    let array = env.new_object_array(2, &class, JObject::null())?;
    env.set_object_array_element(&array, 0, env.byte_array_from_slice(certificate)?)?;
    env.set_object_array_element(&array, 1, env.byte_array_from_slice(private_key)?)?;
    Ok(array)
}

fn null_array<'local>() -> JByteArray<'local> {
    JByteArray::from(JObject::null())
}

fn null_object_array<'local>() -> JObjectArray<'local> {
    JObjectArray::from(JObject::null())
}

fn log(priority: c_int, message: &str) {
    let (Ok(tag), Ok(text)) = (CString::new(LOG_TAG), CString::new(message)) else {
        return;
    };
    // SAFETY: both pointers are valid, NUL-terminated C strings.
    unsafe { __android_log_write(priority, tag.as_ptr(), text.as_ptr()) };
}
