use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: &str = "sim-control/v1";
pub const MAX_FRAME_BYTES: usize = 8192;
pub const MAX_NAME_BYTES: usize = 48;
pub const MAX_TIMEOUT_MS: u64 = 30_000;
pub const SOCKET_PATH_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: String,
    pub id: String,
    pub command: String,
    pub args: Value,
}

#[derive(Debug)]
pub struct Incoming {
    pub stream: UnixStream,
    pub request: Result<Request, String>,
}

#[derive(Serialize)]
struct Response<'a, T: Serialize> {
    version: &'a str,
    id: &'a str,
    ok: bool,
    result: Option<T>,
    error: Option<ErrorBody<'a>>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

pub struct ControlServer {
    listener: UnixListener,
    path: PathBuf,
}

impl ControlServer {
    pub fn bind(root: &Path) -> Result<Self> {
        let path = root.join("control.sock");
        if path.as_os_str().len() > SOCKET_PATH_LIMIT {
            return Err(anyhow!(
                "control socket path is too long ({} bytes); use a shorter run directory",
                path.as_os_str().len()
            ));
        }
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)
            .map_err(|error| anyhow!("create control socket {}: {error}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o660))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path })
    }

    pub fn poll(&self) -> Option<Incoming> {
        let (mut stream, _) = match self.listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(_) => return None,
        };
        let request = read_request(&mut stream);
        Some(Incoming { stream, request })
    }

    pub fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.remove();
    }
}

fn read_request(stream: &mut UnixStream) -> Result<Request, String> {
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut bytes = Vec::with_capacity(256);
    let mut oversized = false;
    let mut one = [0u8; 1];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("control request read timed out".to_string());
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("set control read timeout: {error}"))?;
        match stream.read(&mut one) {
            Ok(0) => break,
            Ok(1) if one[0] == b'\n' => break,
            Ok(1) if !oversized => {
                bytes.push(one[0]);
                if bytes.len() > MAX_FRAME_BYTES {
                    oversized = true;
                    bytes.clear();
                }
            }
            Ok(1) => {}
            Ok(_) => unreachable!(),
            Err(error) => return Err(format!("read control request: {error}")),
        }
    }
    if oversized {
        return Err("request frame exceeds 8192 bytes".to_string());
    }
    if bytes.is_empty() {
        return Err("request frame is empty".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("malformed control request: {error}"))
}

pub fn send_ok<T: Serialize>(stream: &mut UnixStream, id: &str, result: &T) -> Result<()> {
    send(stream, id, true, Some(result), None)
}

pub fn send_error(stream: &mut UnixStream, id: &str, code: &str, message: &str) -> Result<()> {
    send::<Value>(stream, id, false, None, Some(ErrorBody { code, message }))
}

fn send<T: Serialize>(
    stream: &mut UnixStream,
    id: &str,
    ok: bool,
    result: Option<&T>,
    error: Option<ErrorBody<'_>>,
) -> Result<()> {
    let response = Response {
        version: PROTOCOL_VERSION,
        id,
        ok,
        result,
        error,
    };
    let mut bytes = serde_json::to_vec(&response)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}
