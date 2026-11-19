use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message;

use crate::error::AppError;

const VNC_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_VNC_PROXIES: usize = 16;

pub struct VncProxyManager {
    proxies: Mutex<HashMap<String, VncProxyEntry>>,
}

struct VncProxyEntry {
    handle: JoinHandle<()>,
    diagnostics: Arc<Mutex<VncProxyDiagnostics>>,
}

impl VncProxyManager {
    pub fn new() -> Self {
        Self {
            proxies: Mutex::new(HashMap::new()),
        }
    }
}

impl Drop for VncProxyManager {
    fn drop(&mut self) {
        if let Ok(proxies) = self.proxies.lock() {
            for proxy in proxies.values() {
                proxy.handle.abort();
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VncProxy {
    id: String,
    url: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VncProxyDiagnostics {
    stage: VncConnectionStage,
    server_protocol_version: Option<String>,
    offered_security_types: Vec<u8>,
    selected_security_type: Option<u8>,
    failure: Option<VncFailure>,
}

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "snake_case")]
enum VncConnectionStage {
    #[default]
    TcpConnected,
    WebSocketHandshake,
    ProtocolVersion,
    SecurityNegotiation,
    Authentication,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VncFailure {
    code: &'static str,
    message: &'static str,
}

#[derive(Default)]
struct RfbHandshakeObserver {
    server_buffer: Vec<u8>,
    client_buffer: Vec<u8>,
    server_version_seen: bool,
    client_version_seen: bool,
    security_types_seen: bool,
    selected_security_type_seen: bool,
    uses_security_type_list: bool,
    stopped: bool,
}

impl RfbHandshakeObserver {
    fn observe_server(&mut self, bytes: &[u8], diagnostics: &mut VncProxyDiagnostics) {
        if self.stopped || self.security_types_seen {
            return;
        }
        if self.server_buffer.len().saturating_add(bytes.len()) > 512 {
            diagnostics.failure = Some(VncFailure {
                code: "malformed_protocol",
                message: "The server sent an oversized RFB handshake message.",
            });
            self.stopped = true;
            self.server_buffer.clear();
            return;
        }
        self.server_buffer.extend_from_slice(bytes);
        if !self.server_version_seen {
            if self.server_buffer.len() < 12 {
                return;
            }
            let banner: Vec<u8> = self.server_buffer.drain(..12).collect();
            if banner.len() != 12
                || !banner.starts_with(b"RFB ")
                || banner[7] != b'.'
                || banner[11] != b'\n'
                || !banner[4..7].iter().all(u8::is_ascii_digit)
                || !banner[8..11].iter().all(u8::is_ascii_digit)
            {
                diagnostics.failure = Some(VncFailure {
                    code: "unsupported_protocol_version",
                    message: "The server did not provide a supported RFB protocol banner.",
                });
                self.stopped = true;
                self.server_buffer.clear();
                return;
            }
            diagnostics.server_protocol_version =
                Some(String::from_utf8_lossy(&banner[4..11]).into_owned());
            self.uses_security_type_list = &banner[4..11] >= b"003.007".as_slice();
            diagnostics.stage = VncConnectionStage::ProtocolVersion;
            self.server_version_seen = true;
        }

        if !self.client_version_seen || self.server_buffer.is_empty() {
            return;
        }
        if self.uses_security_type_list {
            let count = usize::from(self.server_buffer[0]);
            if count == 0 || self.server_buffer.len() < count + 1 {
                return;
            }
            diagnostics.offered_security_types = self.server_buffer[1..=count].to_vec();
        } else {
            if self.server_buffer.len() < 4 {
                return;
            }
            let security_type = u32::from_be_bytes(
                self.server_buffer[..4]
                    .try_into()
                    .expect("RFB security type length checked"),
            );
            if let Ok(security_type) = u8::try_from(security_type) {
                diagnostics.offered_security_types = vec![security_type];
                diagnostics.selected_security_type = Some(security_type);
                self.selected_security_type_seen = true;
            }
        }
        diagnostics.stage = VncConnectionStage::SecurityNegotiation;
        self.security_types_seen = true;
        self.server_buffer.clear();
    }

    fn observe_client(
        &mut self,
        bytes: &[u8],
        diagnostics: &mut VncProxyDiagnostics,
    ) -> Result<(), AppError> {
        if self.stopped || self.selected_security_type_seen {
            return Ok(());
        }
        if self.client_buffer.len().saturating_add(bytes.len()) > 512 {
            diagnostics.failure = Some(VncFailure {
                code: "malformed_protocol",
                message: "The embedded viewer sent an oversized RFB handshake message.",
            });
            self.stopped = true;
            self.client_buffer.clear();
            return Err(AppError::Validation("Malformed VNC handshake".into()));
        }
        self.client_buffer.extend_from_slice(bytes);
        if !self.client_version_seen {
            if self.client_buffer.len() < 12 {
                return Ok(());
            }
            self.client_buffer.drain(..12);
            self.client_version_seen = true;
            if !self.server_buffer.is_empty() {
                self.observe_server(&[], diagnostics);
            }
        }
        if !self.security_types_seen || self.client_buffer.is_empty() {
            return Ok(());
        }

        let selected = self.client_buffer[0];
        diagnostics.selected_security_type = Some(selected);
        diagnostics.stage = VncConnectionStage::Authentication;
        self.selected_security_type_seen = true;
        self.client_buffer.clear();

        let encrypted_ra2_offered = diagnostics
            .offered_security_types
            .iter()
            .any(|security_type| matches!(security_type, 5 | 13));
        if encrypted_ra2_offered && matches!(selected, 6 | 14) {
            diagnostics.failure = Some(VncFailure {
                code: "encryption_downgrade_blocked",
                message: "The embedded viewer cannot use unencrypted RA2ne when encrypted RA2 is offered.",
            });
            return Err(AppError::Validation(
                "VNC encryption downgrade blocked".into(),
            ));
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn start_vnc_proxy(
    manager: State<'_, VncProxyManager>,
    host: String,
    port: u16,
) -> Result<VncProxy, AppError> {
    start_proxy(&manager, host, port).await
}

async fn start_proxy(
    manager: &VncProxyManager,
    host: String,
    port: u16,
) -> Result<VncProxy, AppError> {
    let host = validate_vnc_host(&host)?.to_string();
    if port == 0 {
        return Err(AppError::Validation(
            "VNC port must be between 1 and 65535".into(),
        ));
    }
    if manager
        .proxies
        .lock()
        .map_err(|_| AppError::Generic("VNC proxy manager lock was poisoned".into()))?
        .len()
        >= MAX_VNC_PROXIES
    {
        return Err(AppError::Validation(format!(
            "VNC_PROXY_LIMIT: No more than {MAX_VNC_PROXIES} embedded VNC sessions may run concurrently"
        )));
    }
    let remote = connect_vnc_remote(&host, port).await?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let local_port = listener.local_addr()?.port();
    let id = uuid::Uuid::new_v4().to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let expected_path = format!("/vnc/{token}");
    let diagnostics = Arc::new(Mutex::new(VncProxyDiagnostics::default()));
    let listener_diagnostics = Arc::clone(&diagnostics);
    let handle = tokio::spawn(async move {
        let initial_remote = Arc::new(tokio::sync::Mutex::new(Some(remote)));
        let connection_claimed = Arc::new(AtomicBool::new(false));
        let mut connections = JoinSet::new();
        loop {
            let (socket, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    log::warn!("VNC loopback listener stopped: {error}");
                    return;
                }
            };
            let expected_path = expected_path.clone();
            let host = host.clone();
            let initial_remote = Arc::clone(&initial_remote);
            let connection_claimed = Arc::clone(&connection_claimed);
            let diagnostics = Arc::clone(&listener_diagnostics);
            connections.spawn(async move {
                if let Err(error) = proxy_connection(
                    socket,
                    &expected_path,
                    &host,
                    port,
                    initial_remote,
                    connection_claimed,
                    diagnostics,
                )
                .await
                {
                    log::warn!("VNC bridge connection ended: {error}");
                }
            });
        }
    });
    let mut proxies = manager
        .proxies
        .lock()
        .map_err(|_| AppError::Generic("VNC proxy manager lock was poisoned".into()))?;
    if proxies.len() >= MAX_VNC_PROXIES {
        handle.abort();
        return Err(AppError::Validation(format!(
            "VNC_PROXY_LIMIT: No more than {MAX_VNC_PROXIES} embedded VNC sessions may run concurrently"
        )));
    }
    proxies.insert(
        id.clone(),
        VncProxyEntry {
            handle,
            diagnostics,
        },
    );
    Ok(VncProxy {
        id,
        url: format!("ws://127.0.0.1:{local_port}/vnc/{token}"),
    })
}

async fn connect_vnc_remote(host: &str, port: u16) -> Result<TcpStream, AppError> {
    let addresses =
        tokio::time::timeout(VNC_CONNECT_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| {
                AppError::Generic("VNC_DNS_TIMEOUT: VNC hostname resolution timed out".into())
            })?
            .map_err(|_| AppError::Generic("VNC_DNS: Unable to resolve the VNC hostname".into()))?
            .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(AppError::Generic(
            "VNC_DNS: The VNC hostname did not resolve to an address".into(),
        ));
    }

    let deadline = tokio::time::Instant::now() + VNC_CONNECT_TIMEOUT;
    let mut last_error_kind = None;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last_error_kind = Some(error.kind()),
            Err(_) => break,
        }
    }

    match last_error_kind {
        Some(std::io::ErrorKind::ConnectionRefused) => Err(AppError::Generic(
            "VNC_TCP_REFUSED: The VNC server refused the TCP connection".into(),
        )),
        Some(std::io::ErrorKind::PermissionDenied) => Err(AppError::Generic(
            "VNC_TCP_BLOCKED: The operating system blocked the VNC connection".into(),
        )),
        Some(_) => Err(AppError::Generic(
            "VNC_TCP: Unable to establish a TCP connection to the VNC server".into(),
        )),
        None => Err(AppError::Generic(
            "VNC_TCP_TIMEOUT: The VNC TCP connection timed out".into(),
        )),
    }
}

#[tauri::command]
pub fn stop_vnc_proxy(manager: State<'_, VncProxyManager>, id: String) -> Result<(), AppError> {
    stop_proxy(&manager, &id)
}

fn stop_proxy(manager: &VncProxyManager, id: &str) -> Result<(), AppError> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::Validation("Invalid VNC proxy identifier".into()))?;
    if let Some(proxy) = manager
        .proxies
        .lock()
        .map_err(|_| AppError::Generic("VNC proxy manager lock was poisoned".into()))?
        .remove(id)
    {
        proxy.handle.abort();
    }
    Ok(())
}

#[tauri::command]
pub fn get_vnc_proxy_diagnostics(
    manager: State<'_, VncProxyManager>,
    id: String,
) -> Result<VncProxyDiagnostics, AppError> {
    uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::Validation("Invalid VNC proxy identifier".into()))?;
    let diagnostics = Arc::clone(
        &manager
            .proxies
            .lock()
            .map_err(|_| AppError::Generic("VNC proxy manager lock was poisoned".into()))?
            .get(&id)
            .ok_or_else(|| AppError::NotFound("VNC proxy session".into()))?
            .diagnostics,
    );
    let snapshot = diagnostics
        .lock()
        .map_err(|_| AppError::Generic("VNC diagnostics lock was poisoned".into()))?
        .clone();
    Ok(snapshot)
}

#[allow(clippy::result_large_err)]
async fn proxy_connection(
    socket: TcpStream,
    expected_path: &str,
    host: &str,
    port: u16,
    initial_remote: Arc<tokio::sync::Mutex<Option<TcpStream>>>,
    connection_claimed: Arc<AtomicBool>,
    diagnostics: Arc<Mutex<VncProxyDiagnostics>>,
) -> Result<(), AppError> {
    if let Ok(mut snapshot) = diagnostics.lock() {
        snapshot.stage = VncConnectionStage::WebSocketHandshake;
    }
    let websocket =
        tokio_tungstenite::accept_hdr_async(socket, |request: &Request, response: Response| {
            if request.uri().path() == expected_path && allowed_webview_origin(request) {
                Ok(response)
            } else {
                Err(ErrorResponse::new(Some("Not found".into())))
            }
        })
        .await
        .map_err(|error| {
            if let Ok(mut snapshot) = diagnostics.lock() {
                snapshot.failure = Some(VncFailure {
                    code: "websocket_handshake_failed",
                    message: "The local VNC WebSocket handshake failed.",
                });
            }
            AppError::Generic(format!("VNC WebSocket handshake failed: {error}"))
        })?;
    if connection_claimed.swap(true, Ordering::AcqRel) {
        return Err(AppError::Validation(
            "VNC proxy already has an authorized viewer".into(),
        ));
    }
    let remote = match initial_remote.lock().await.take() {
        Some(remote) => remote,
        None => connect_vnc_remote(host, port).await?,
    };
    let (mut websocket_write, mut websocket_read) = websocket.split();
    let (mut remote_read, mut remote_write) = remote.into_split();
    let mut remote_buffer = vec![0_u8; 64 * 1024];
    let mut observer = RfbHandshakeObserver::default();

    loop {
        tokio::select! {
            message = websocket_read.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Ok(mut snapshot) = diagnostics.lock() {
                        observer.observe_client(&bytes, &mut snapshot)?;
                    }
                    remote_write.write_all(&bytes).await?
                },
                Some(Ok(Message::Ping(bytes))) => websocket_write.send(Message::Pong(bytes)).await
                    .map_err(|error| AppError::Generic(format!("VNC WebSocket write failed: {error}")))?,
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => return Err(AppError::Validation("VNC bridge accepts binary WebSocket frames only".into())),
                Some(Err(error)) => return Err(AppError::Generic(format!("VNC WebSocket read failed: {error}"))),
            },
            read = remote_read.read(&mut remote_buffer) => {
                let read = read?;
                if read == 0 {
                    break;
                }
                if let Ok(mut snapshot) = diagnostics.lock() {
                    observer.observe_server(&remote_buffer[..read], &mut snapshot);
                }
                websocket_write.send(Message::Binary(remote_buffer[..read].to_vec().into())).await
                    .map_err(|error| AppError::Generic(format!("VNC WebSocket write failed: {error}")))?;
            }
        }
    }
    Ok(())
}

fn allowed_webview_origin(request: &Request) -> bool {
    request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            let allowed = matches!(
                origin,
                "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
            );
            #[cfg(debug_assertions)]
            let allowed = allowed || origin == "http://localhost:1420";
            allowed
        })
}

fn validate_vnc_host(host: &str) -> Result<&str, AppError> {
    let host = host.trim();
    if host.is_empty()
        || host.starts_with('-')
        || host.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '/' | '\\' | '@' | '#')
        })
    {
        return Err(AppError::Validation(
            "Enter a valid VNC hostname or IP address".into(),
        ));
    }
    Ok(host)
}

#[tauri::command]
pub fn launch_vnc_viewer(host: String, port: u16) -> Result<(), AppError> {
    let endpoint = vnc_endpoint(&host, port)?;
    let executable = find_vnc_viewer().ok_or_else(|| {
        AppError::Generic(
            "RealVNC Viewer was not found. Install VNC Viewer from realvnc.com, then try again."
                .to_string(),
        )
    })?;

    Command::new(&executable)
        .args(viewer_args(endpoint))
        .spawn()
        .map_err(|error| {
            AppError::Generic(format!(
                "Failed to launch RealVNC Viewer at {}: {error}",
                executable.display(),
            ))
        })?;
    Ok(())
}

fn viewer_args(endpoint: String) -> [String; 2] {
    ["-useaddressbook".to_string(), endpoint]
}
fn vnc_endpoint(host: &str, port: u16) -> Result<String, AppError> {
    let host = validate_vnc_host(host)?;
    if port == 0 {
        return Err(AppError::Generic(
            "VNC port must be between 1 and 65535.".to_string(),
        ));
    }

    let formatted_host = if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    Ok(format!("{formatted_host}::{port}"))
}

fn find_vnc_viewer() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(root).join("RealVNC/VNC Viewer/vncviewer.exe"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join("vncviewer.exe"))
                    .find(|candidate| candidate.is_file())
            })
        })
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    use super::{
        start_proxy, stop_proxy, viewer_args, vnc_endpoint, RfbHandshakeObserver,
        VncProxyDiagnostics, VncProxyManager,
    };

    #[test]
    fn observes_rfb_version_and_security_selection_without_payloads() {
        let mut observer = RfbHandshakeObserver::default();
        let mut diagnostics = VncProxyDiagnostics::default();

        observer.observe_server(b"RFB 003.008\n", &mut diagnostics);
        observer
            .observe_client(b"RFB 003.008\n", &mut diagnostics)
            .unwrap();
        observer.observe_server(&[2, 2, 6], &mut diagnostics);
        observer.observe_client(&[6], &mut diagnostics).unwrap();
        observer.observe_server(b"authentication payload", &mut diagnostics);

        assert_eq!(
            diagnostics.server_protocol_version.as_deref(),
            Some("003.008")
        );
        assert_eq!(diagnostics.offered_security_types, vec![2, 6]);
        assert_eq!(diagnostics.selected_security_type, Some(6));
    }

    #[test]
    fn observes_rfb_33_server_selected_security_type() {
        let mut observer = RfbHandshakeObserver::default();
        let mut diagnostics = VncProxyDiagnostics::default();

        observer.observe_server(b"RFB 003.003\n", &mut diagnostics);
        observer
            .observe_client(b"RFB 003.003\n", &mut diagnostics)
            .unwrap();
        observer.observe_server(&[0, 0, 0, 2], &mut diagnostics);

        assert_eq!(diagnostics.offered_security_types, vec![2]);
        assert_eq!(diagnostics.selected_security_type, Some(2));
    }

    #[test]
    fn bounds_malformed_handshake_metadata() {
        let mut observer = RfbHandshakeObserver::default();
        let mut diagnostics = VncProxyDiagnostics::default();

        observer.observe_server(&vec![0; 513], &mut diagnostics);

        assert!(observer.server_buffer.is_empty());
        assert_eq!(
            diagnostics.failure.as_ref().map(|failure| failure.code),
            Some("malformed_protocol")
        );
    }

    #[test]
    fn blocks_downgrade_when_encrypted_realvnc_security_is_offered() {
        let mut observer = RfbHandshakeObserver::default();
        let mut diagnostics = VncProxyDiagnostics::default();

        observer.observe_server(b"RFB 003.008\n", &mut diagnostics);
        observer
            .observe_client(b"RFB 003.008\n", &mut diagnostics)
            .unwrap();
        observer.observe_server(&[2, 5, 6], &mut diagnostics);
        let error = observer.observe_client(&[6], &mut diagnostics).unwrap_err();

        assert!(error.to_string().contains("downgrade blocked"));
        assert_eq!(
            diagnostics.failure.as_ref().map(|failure| failure.code),
            Some("encryption_downgrade_blocked")
        );
    }

    #[test]
    fn blocks_ra2ne_256_downgrade_when_encrypted_ra2_256_is_offered() {
        let mut observer = RfbHandshakeObserver::default();
        let mut diagnostics = VncProxyDiagnostics::default();

        observer.observe_server(b"RFB 003.008\n", &mut diagnostics);
        observer
            .observe_client(b"RFB 003.008\n", &mut diagnostics)
            .unwrap();
        observer.observe_server(&[2, 13, 14], &mut diagnostics);
        let error = observer
            .observe_client(&[14], &mut diagnostics)
            .unwrap_err();

        assert!(error.to_string().contains("downgrade blocked"));
    }

    #[test]
    fn classifies_invalid_rfb_protocol_banner() {
        let mut observer = RfbHandshakeObserver::default();
        let mut diagnostics = VncProxyDiagnostics::default();

        observer.observe_server(b"HTTP/1.1 200", &mut diagnostics);

        assert_eq!(
            diagnostics.failure.as_ref().map(|failure| failure.code),
            Some("unsupported_protocol_version")
        );
    }

    #[test]
    fn formats_vnc_endpoints_without_exposing_credentials() {
        assert_eq!(
            vnc_endpoint("vm.example", 5900).unwrap(),
            "vm.example::5900"
        );
        assert_eq!(
            vnc_endpoint("2001:db8::10", 5901).unwrap(),
            "[2001:db8::10]::5901"
        );
    }

    #[test]
    fn rejects_option_like_or_empty_hosts() {
        assert!(vnc_endpoint("", 5900).is_err());
        assert!(vnc_endpoint("--help", 5900).is_err());
        assert!(vnc_endpoint("bad host", 5900).is_err());
        assert!(vnc_endpoint("vm.example", 0).is_err());
    }

    #[test]
    fn uses_address_book_to_reuse_remembered_credentials() {
        assert_eq!(
            viewer_args("vm.example::5900".to_string()),
            ["-useaddressbook", "vm.example::5900"],
        );
    }

    #[tokio::test]
    async fn bridges_binary_websocket_frames_to_tcp() {
        let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let remote_port = remote.local_addr().unwrap().port();
        let echo = tokio::spawn(async move {
            let (mut socket, _) = remote.accept().await.unwrap();
            let mut bytes = [0_u8; 4];
            socket.read_exact(&mut bytes).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        });
        let manager = VncProxyManager::new();
        let proxy = start_proxy(&manager, "127.0.0.1".into(), remote_port)
            .await
            .unwrap();
        let mut request = proxy.url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", "http://tauri.localhost".parse().unwrap());
        let (mut websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        websocket
            .send(Message::Binary(vec![1, 2, 3, 4].into()))
            .await
            .unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(2), websocket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.into_data().as_ref(), &[1, 2, 3, 4]);
        echo.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_unapproved_origin_without_consuming_remote_connection() {
        let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let remote_port = remote.local_addr().unwrap().port();
        let echo = tokio::spawn(async move {
            let (mut socket, _) = remote.accept().await.unwrap();
            let mut byte = [0_u8; 1];
            socket.read_exact(&mut byte).await.unwrap();
            socket.write_all(&byte).await.unwrap();
        });
        let manager = VncProxyManager::new();
        let proxy = start_proxy(&manager, "127.0.0.1".into(), remote_port)
            .await
            .unwrap();

        let mut rejected_request = proxy.url.clone().into_client_request().unwrap();
        rejected_request
            .headers_mut()
            .insert("origin", "https://example.invalid".parse().unwrap());
        assert!(tokio_tungstenite::connect_async(rejected_request)
            .await
            .is_err());

        let mut approved_request = proxy.url.into_client_request().unwrap();
        approved_request
            .headers_mut()
            .insert("origin", "http://tauri.localhost".parse().unwrap());
        let (mut websocket, _) = tokio_tungstenite::connect_async(approved_request)
            .await
            .unwrap();
        websocket
            .send(Message::Binary(vec![42].into()))
            .await
            .unwrap();
        let response = websocket.next().await.unwrap().unwrap();
        assert_eq!(response.into_data().as_ref(), &[42]);
        echo.await.unwrap();
    }

    #[tokio::test]
    async fn accepts_only_one_authorized_viewer_per_proxy() {
        let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let remote_port = remote.local_addr().unwrap().port();
        let manager = VncProxyManager::new();
        let proxy = start_proxy(&manager, "127.0.0.1".into(), remote_port)
            .await
            .unwrap();
        let (_remote_socket, _) = remote.accept().await.unwrap();

        let mut first_request = proxy.url.clone().into_client_request().unwrap();
        first_request
            .headers_mut()
            .insert("origin", "http://tauri.localhost".parse().unwrap());
        let (_first, _) = tokio_tungstenite::connect_async(first_request)
            .await
            .unwrap();

        let mut second_request = proxy.url.into_client_request().unwrap();
        second_request
            .headers_mut()
            .insert("origin", "http://tauri.localhost".parse().unwrap());
        let (mut second, _) = tokio_tungstenite::connect_async(second_request)
            .await
            .unwrap();
        let closed = tokio::time::timeout(std::time::Duration::from_secs(2), second.next())
            .await
            .unwrap();
        assert!(closed.is_none() || closed.is_some_and(|message| message.is_err()));
    }

    #[tokio::test]
    async fn stopping_proxy_closes_active_bridge_connection() {
        let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let remote_port = remote.local_addr().unwrap().port();
        let manager = VncProxyManager::new();
        let proxy = start_proxy(&manager, "127.0.0.1".into(), remote_port)
            .await
            .unwrap();
        let (mut remote_socket, _) = remote.accept().await.unwrap();
        let mut request = proxy.url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", "http://tauri.localhost".parse().unwrap());
        let (_websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

        stop_proxy(&manager, &proxy.id).unwrap();

        let mut byte = [0_u8; 1];
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_socket.read(&mut byte),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(read, 0);
    }

    #[tokio::test]
    async fn reports_unavailable_remote_before_starting_proxy() {
        let unavailable = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = unavailable.local_addr().unwrap().port();
        drop(unavailable);

        let error = match start_proxy(&VncProxyManager::new(), "127.0.0.1".into(), port).await {
            Ok(_) => panic!("an unavailable endpoint must fail proxy startup"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("VNC_TCP_REFUSED"));
    }
}
