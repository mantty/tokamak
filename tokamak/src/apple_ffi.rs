#![deny(missing_docs)]

//! C ABI used by the native Apple application shell.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr;

use crate::packaging::PackageLayout;
use crate::{Challenge, Config, Decision, Event, Runtime};
use p256::{SecretKey, pkcs8::DecodePrivateKey};

const DECISION_DEFAULT: c_int = 0;
const DECISION_CANCEL: c_int = 1;
const DECISION_USE: c_int = 2;

/// An owned byte buffer returned across the C ABI.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TokamakBytes {
    /// Buffer start, or null when empty.
    pub data: *mut u8,
    /// Buffer length.
    pub len: usize,
}

/// Client certificate material returned across the C ABI.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TokamakIdentity {
    /// X.509 client certificate in DER.
    pub certificate: TokamakBytes,
    /// EC private key in ANSI X9.63 form.
    pub private_key: TokamakBytes,
}

/// Start a tokamak runtime.
///
/// # Safety
///
/// All string pointers must reference NUL-terminated UTF-8 strings. `error`
/// must be writable for `error_len` bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_start(
    packaged_dir: *const c_char,
    state_dir: *const c_char,
    host: *const c_char,
    error: *mut c_char,
    error_len: usize,
) -> *mut c_void {
    let result = unsafe { start(packaged_dir, state_dir, host) };
    match result {
        Ok(runtime) => Box::into_raw(Box::new(runtime)).cast(),
        Err(message) => {
            write_error(error, error_len, &message);
            ptr::null_mut()
        }
    }
}

/// Start a tokamak runtime that forwards requests to a host development server.
///
/// # Safety
///
/// All string pointers must reference NUL-terminated UTF-8 strings. `error`
/// must be writable for `error_len` bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_start_development(
    state_dir: *const c_char,
    host: *const c_char,
    endpoint: *const c_char,
    session_token: *const c_char,
    error: *mut c_char,
    error_len: usize,
) -> *mut c_void {
    let result = unsafe { start_development(state_dir, host, endpoint, session_token) };
    match result {
        Ok(runtime) => Box::into_raw(Box::new(runtime)).cast(),
        Err(message) => {
            write_error(error, error_len, &message);
            ptr::null_mut()
        }
    }
}

/// Return the runtime's loopback port.
///
/// # Safety
///
/// `handle` must be null or a live handle returned by [`tokamak_runtime_start`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_port(handle: *const c_void) -> u16 {
    unsafe { runtime(handle) }.map_or(0, Runtime::port)
}

/// Wait for the runtime's loopback gateway and return its current port.
///
/// # Safety
///
/// `handle` must be null or a live handle returned by [`tokamak_runtime_start`].
/// `error` must be writable for `error_len` bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_restore_gateway(
    handle: *const c_void,
    error: *mut c_char,
    error_len: usize,
) -> u16 {
    let result = unsafe { runtime(handle) }
        .ok_or_else(|| "runtime is unavailable".to_owned())
        .and_then(|runtime| runtime.restore_gateway().map_err(|error| error.to_string()));
    match result {
        Ok(port) => port,
        Err(message) => {
            write_error(error, error_len, &message);
            0
        }
    }
}

/// Suspend JavaScript execution; returns whether `handle` was live.
///
/// # Safety
///
/// `handle` must be null or a live handle returned by [`tokamak_runtime_start`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_suspend(handle: *const c_void) -> bool {
    unsafe { runtime(handle) }.map(Runtime::suspend).is_some()
}

/// Resume JavaScript execution.
///
/// # Safety
///
/// `handle` must be null or a live handle returned by [`tokamak_runtime_start`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_resume(handle: *const c_void) -> bool {
    unsafe { runtime(handle) }.is_some_and(|runtime| runtime.resume().is_ok())
}

/// Stop a runtime and release its handle.
///
/// # Safety
///
/// `handle` must be null or a live, unconsumed handle returned by
/// [`tokamak_runtime_start`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_stop(handle: *mut c_void) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle.cast::<Runtime>()) });
    }
}

/// Decide a server-trust challenge and return its authority when trusted.
///
/// # Safety
///
/// `handle` must be live, `host` must be a NUL-terminated UTF-8 string, and
/// `authority` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_server_authority(
    handle: *const c_void,
    host: *const c_char,
    authority: *mut TokamakBytes,
) -> c_int {
    if authority.is_null() {
        return DECISION_CANCEL;
    }
    unsafe { authority.write(TokamakBytes::empty()) };
    let Some(runtime) = (unsafe { runtime(handle) }) else {
        return DECISION_CANCEL;
    };
    let Some(host) = (unsafe { text(host) }) else {
        return DECISION_CANCEL;
    };
    match runtime
        .certificates()
        .decide(&Challenge::ServerTrust { host })
    {
        Decision::PerformDefault => DECISION_DEFAULT,
        Decision::Cancel | Decision::PresentIdentity { .. } => DECISION_CANCEL,
        Decision::TrustAuthority(bytes) => {
            unsafe { authority.write(TokamakBytes::from_vec(bytes)) };
            DECISION_USE
        }
    }
}

/// Decide a client-certificate challenge and return the selected identity.
///
/// # Safety
///
/// `handle` must be live, `host` must be a NUL-terminated UTF-8 string, and
/// `identity` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_runtime_client_identity(
    handle: *const c_void,
    host: *const c_char,
    previous_failures: usize,
    identity: *mut TokamakIdentity,
) -> c_int {
    if identity.is_null() {
        return DECISION_CANCEL;
    }
    unsafe { identity.write(TokamakIdentity::empty()) };
    let Some(runtime) = (unsafe { runtime(handle) }) else {
        return DECISION_CANCEL;
    };
    let Some(host) = (unsafe { text(host) }) else {
        return DECISION_CANCEL;
    };
    match runtime
        .certificates()
        .decide(&Challenge::ClientCertificate {
            host,
            previous_failures,
        }) {
        Decision::PerformDefault => DECISION_DEFAULT,
        Decision::Cancel | Decision::TrustAuthority(_) => DECISION_CANCEL,
        Decision::PresentIdentity {
            certificate,
            private_key,
        } => {
            let Some(private_key) = x963_private_key(&private_key) else {
                return DECISION_CANCEL;
            };
            unsafe {
                identity.write(TokamakIdentity {
                    certificate: TokamakBytes::from_vec(certificate),
                    private_key: TokamakBytes::from_vec(private_key),
                });
            }
            DECISION_USE
        }
    }
}

/// Release a byte buffer returned by this library.
///
/// # Safety
///
/// `bytes` must be empty or an unconsumed value returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_bytes_free(bytes: TokamakBytes) {
    if bytes.data.is_null() {
        return;
    }
    let slice = ptr::slice_from_raw_parts_mut(bytes.data, bytes.len);
    drop(unsafe { Box::from_raw(slice) });
}

/// Release an identity returned by this library.
///
/// # Safety
///
/// `identity` must be empty or an unconsumed value returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tokamak_identity_free(identity: TokamakIdentity) {
    unsafe {
        tokamak_bytes_free(identity.certificate);
        tokamak_bytes_free(identity.private_key);
    }
}

unsafe fn start(
    packaged_dir: *const c_char,
    state_dir: *const c_char,
    host: *const c_char,
) -> Result<Runtime, String> {
    let config = Config {
        app: PackageLayout::new(
            unsafe { text(packaged_dir) }.ok_or("packaged app path is not valid UTF-8")?,
        ),
        state_dir: PathBuf::from(
            unsafe { text(state_dir) }.ok_or("state directory is not valid UTF-8")?,
        ),
        host: unsafe { text(host) }
            .ok_or("app host is not valid UTF-8")?
            .to_owned(),
    };
    Runtime::start(config, report).map_err(|error| error.to_string())
}

unsafe fn start_development(
    state_dir: *const c_char,
    host: *const c_char,
    endpoint: *const c_char,
    session_token: *const c_char,
) -> Result<Runtime, String> {
    let config = crate::DevelopmentConfig {
        state_dir: PathBuf::from(
            unsafe { text(state_dir) }.ok_or("development state path is not valid UTF-8")?,
        ),
        host: unsafe { text(host) }
            .ok_or("app host is not valid UTF-8")?
            .to_owned(),
        proxy: crate::DevProxyConfig {
            endpoint: unsafe { text(endpoint) }
                .ok_or("development endpoint is not valid UTF-8")?
                .to_owned(),
            session_token: unsafe { text(session_token) }
                .ok_or("development session token is not valid UTF-8")?
                .to_owned(),
        },
    };
    Runtime::start_development(config, report).map_err(|error| error.to_string())
}

fn report(event: Event) {
    match event {
        Event::Starting => eprintln!("tokamak runtime starting"),
        Event::Listening { port } => eprintln!("tokamak gateway listening on 127.0.0.1:{port}"),
        Event::Suspended => eprintln!("tokamak runtime suspended"),
        Event::Resumed => eprintln!("tokamak runtime resumed"),
        Event::CertificatesRenewed => eprintln!("tokamak certificates renewed"),
        Event::Failed { message } => eprintln!("tokamak runtime failed: {message}"),
        Event::RequestFailed { message } => eprintln!("tokamak request failed: {message}"),
    }
}

unsafe fn runtime<'handle>(handle: *const c_void) -> Option<&'handle Runtime> {
    unsafe { handle.cast::<Runtime>().as_ref() }
}

unsafe fn text<'value>(value: *const c_char) -> Option<&'value str> {
    if value.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(value) }.to_str().ok()
}

fn write_error(output: *mut c_char, capacity: usize, message: &str) {
    if output.is_null() || capacity == 0 {
        return;
    }
    let bytes = message.as_bytes();
    let count = bytes.len().min(capacity - 1);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), output.cast(), count);
        output.add(count).write(0);
    }
}

fn x963_private_key(pkcs8_der: &[u8]) -> Option<Vec<u8>> {
    let secret = SecretKey::from_pkcs8_der(pkcs8_der).ok()?;
    let public = secret.public_key().to_sec1_bytes();
    Some([&public[..], &secret.to_bytes()[..]].concat())
}

impl TokamakBytes {
    const fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }

    fn from_vec(bytes: Vec<u8>) -> Self {
        let mut bytes = bytes.into_boxed_slice();
        let value = Self {
            data: bytes.as_mut_ptr(),
            len: bytes.len(),
        };
        std::mem::forget(bytes);
        value
    }
}

impl TokamakIdentity {
    const fn empty() -> Self {
        Self {
            certificate: TokamakBytes::empty(),
            private_key: TokamakBytes::empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TokamakBytes, write_error, x963_private_key};

    #[test]
    fn writes_a_terminated_bounded_error() {
        let mut output = [1_i8; 5];

        write_error(output.as_mut_ptr(), output.len(), "longer");

        assert_eq!(output, [108, 111, 110, 103, 0]);
    }

    #[test]
    fn converts_a_pkcs8_key_to_the_x963_layout() -> Result<(), Box<dyn std::error::Error>> {
        let key = rcgen::KeyPair::generate()?;

        let converted = x963_private_key(key.serialized_der())
            .ok_or_else(|| std::io::Error::other("key should convert"))?;

        assert_eq!(converted.len(), 65 + 32);
        assert_eq!(converted[0], 0x04);
        Ok(())
    }

    #[test]
    fn owns_and_releases_returned_bytes() {
        let bytes = TokamakBytes::from_vec(vec![1, 2, 3]);

        assert_eq!(
            unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) },
            [1, 2, 3]
        );
        unsafe { super::tokamak_bytes_free(bytes) };
    }
}
