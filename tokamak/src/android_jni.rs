//! The JNI seam between the Kotlin shell and the tokamak runtime.
//!
//! The shell owns the Activity, `WebView`, and lifecycle. This crate only
//! moves values across the boundary; every decision belongs to the runtime.

use std::ffi::{CString, c_char, c_int};
use std::path::PathBuf;

use crate::packaging::PackageLayout;
use crate::{Challenge, Config, Decision, Event, Runtime};
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JObjectArray, JString};
use jni::sys::{jint, jlong};

const LOG_TAG: &str = "tokamak";
const LOG_INFO: c_int = 4;
const LOG_ERROR: c_int = 6;
const FAILURE: &str = "java/lang/IllegalStateException";

unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Start the runtime and return an opaque handle, or throw on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeStart(
    mut env: JNIEnv,
    _: JClass,
    packaged_dir: JString,
    state_dir: JString,
    host: JString,
) -> jlong {
    match start(&mut env, &packaged_dir, &state_dir, &host) {
        Ok(runtime) => Box::into_raw(Box::new(runtime)) as jlong,
        Err(message) => {
            log(LOG_ERROR, &format!("runtime startup failed: {message}"));
            let _ = env.throw_new(FAILURE, message);
            0
        }
    }
}

/// Start the development runtime and return an opaque handle, or throw on
/// failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeStartDevelopment(
    mut env: JNIEnv,
    _: JClass,
    state_dir: JString,
    host: JString,
    endpoint: JString,
    session_token: JString,
) -> jlong {
    match start_development(&mut env, &state_dir, &host, &endpoint, &session_token) {
        Ok(runtime) => Box::into_raw(Box::new(runtime)) as jlong,
        Err(message) => {
            log(
                LOG_ERROR,
                &format!("development runtime startup failed: {message}"),
            );
            let _ = env.throw_new(FAILURE, message);
            0
        }
    }
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

/// Suspend JavaScript execution.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeSuspend(
    _: JNIEnv,
    _: JClass,
    handle: jlong,
) {
    if let Some(runtime) = runtime(handle) {
        runtime.suspend();
    }
}

/// Resume JavaScript execution.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeResume(
    _: JNIEnv,
    _: JClass,
    handle: jlong,
) {
    if let Some(runtime) = runtime(handle)
        && let Err(error) = runtime.resume()
    {
        log(LOG_ERROR, &format!("resume failed: {error}"));
    }
}

/// Stop the runtime and release the handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_tokamak_runtime_TokamakRuntime_nativeStop(
    _: JNIEnv,
    _: JClass,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    // SAFETY: `handle` came from `Box::into_raw` in `nativeStart` and the
    // Kotlin shell stops a runtime once.
    drop(unsafe { Box::from_raw(handle as *mut Runtime) });
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
        Decision::TrustAuthority(authority) => bytes(&env, &authority),
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
    host: &JString,
) -> Result<Runtime, String> {
    let config = Config {
        app: PackageLayout::new(text(env, packaged_dir)?),
        state_dir: PathBuf::from(text(env, state_dir)?),
        host: text(env, host)?,
    };
    Runtime::start(config, report).map_err(|error| error.to_string())
}

fn start_development(
    env: &mut JNIEnv,
    state_dir: &JString,
    host: &JString,
    endpoint: &JString,
    session_token: &JString,
) -> Result<Runtime, String> {
    let config = crate::DevelopmentConfig {
        state_dir: PathBuf::from(text(env, state_dir)?),
        host: text(env, host)?,
        proxy: crate::DevProxyConfig {
            endpoint: text(env, endpoint)?,
            session_token: text(env, session_token)?,
        },
    };
    Runtime::start_development(config, report).map_err(|error| error.to_string())
}

fn report(event: Event) {
    match event {
        Event::Starting => log(LOG_INFO, "runtime starting"),
        Event::Listening { port } => log(LOG_INFO, &format!("gateway listening on {port}")),
        Event::Suspended => log(LOG_INFO, "runtime suspended"),
        Event::Resumed => log(LOG_INFO, "runtime resumed"),
        Event::CertificatesRenewed => log(LOG_INFO, "certificates renewed"),
        Event::Failed { message } => log(LOG_ERROR, &format!("runtime failed: {message}")),
    }
}

fn runtime<'handle>(handle: jlong) -> Option<&'handle Runtime> {
    if handle == 0 {
        return None;
    }
    // SAFETY: `handle` came from `Box::into_raw` in `nativeStart` and stays
    // valid until `nativeStop`.
    Some(unsafe { &*(handle as *const Runtime) })
}

fn text(env: &mut JNIEnv, value: &JString) -> Result<String, String> {
    env.get_string(value)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

fn bytes<'local>(env: &JNIEnv<'local>, data: &[u8]) -> JByteArray<'local> {
    env.byte_array_from_slice(data)
        .unwrap_or_else(|_| null_array())
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
