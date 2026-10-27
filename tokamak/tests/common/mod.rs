//! Connection helpers the gateway integration tests share.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn read_header_block(stream: &mut impl Read) -> TestResult<Vec<u8>> {
    let mut data = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        data.push(byte[0]);
        if data.ends_with(b"\r\n\r\n") {
            return Ok(data);
        }
    }
}

/// Writes a WebSocket frame as a client sends it, masked.
pub fn write_masked_frame(
    stream: &mut impl Write,
    final_frame: bool,
    opcode: u8,
    payload: &[u8],
) -> TestResult {
    let mask = [1, 2, 3, 4];
    let mut frame = vec![(if final_frame { 0x80 } else { 0 }) | opcode];
    match payload.len() {
        length @ ..=125 => frame.push(0x80 | u8::try_from(length)?),
        length @ ..=0xFFFF => {
            frame.push(0x80 | 0x7E);
            frame.extend_from_slice(&u16::try_from(length)?.to_be_bytes());
        }
        length => {
            frame.push(0x80 | 0x7F);
            frame.extend_from_slice(&u64::try_from(length)?.to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    stream.write_all(&frame)?;
    stream.flush()?;
    Ok(())
}

/// A TLS connection over `stream` that trusts the runtime's authority in
/// `state` and presents its client certificate.
pub fn connect_tls(
    state: &Path,
    host: &str,
    stream: TcpStream,
) -> TestResult<StreamOwned<ClientConnection, TcpStream>> {
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_file_iter(state.join("ca.cert.pem"))? {
        roots.add(certificate?)?;
    }
    let chain = CertificateDer::pem_file_iter(state.join("client.cert.pem"))?
        .collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(state.join("client.key.pem"))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)?;
    let connection =
        ClientConnection::new(Arc::new(config), ServerName::try_from(host.to_owned())?)?;
    let mut tls = StreamOwned::new(connection, stream);
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock)?;
    }
    Ok(tls)
}

/// Makes closing `stream` reset the connection instead of ending it.
#[cfg(unix)]
pub fn reset_connection(stream: &TcpStream) -> TestResult {
    use std::os::fd::AsRawFd;

    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    let linger_size = libc::socklen_t::try_from(std::mem::size_of_val(&linger))?;
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&raw const linger).cast(),
            linger_size,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}
