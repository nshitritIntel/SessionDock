use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message;

use crate::error::AppError;

pub struct VncProxyManager {
    proxies: Mutex<HashMap<String, JoinHandle<()>>>,
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
            for handle in proxies.values() {
                handle.abort();
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
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let local_port = listener.local_addr()?.port();
    let id = uuid::Uuid::new_v4().to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let expected_path = format!("/vnc/{token}");
    let handle = tokio::spawn(async move {
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
            tokio::spawn(async move {
                if let Err(error) = proxy_connection(socket, &expected_path, &host, port).await {
                    log::warn!("VNC bridge connection ended: {error}");
                }
            });
        }
    });
    manager
        .proxies
        .lock()
        .map_err(|_| AppError::Generic("VNC proxy manager lock was poisoned".into()))?
        .insert(id.clone(), handle);
    Ok(VncProxy {
        id,
        url: format!("ws://127.0.0.1:{local_port}/vnc/{token}"),
    })
}

#[tauri::command]
pub fn stop_vnc_proxy(manager: State<'_, VncProxyManager>, id: String) -> Result<(), AppError> {
    uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::Validation("Invalid VNC proxy identifier".into()))?;
    if let Some(handle) = manager
        .proxies
        .lock()
        .map_err(|_| AppError::Generic("VNC proxy manager lock was poisoned".into()))?
        .remove(&id)
    {
        handle.abort();
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
async fn proxy_connection(
    socket: TcpStream,
    expected_path: &str,
    host: &str,
    port: u16,
) -> Result<(), AppError> {
    let websocket =
        tokio_tungstenite::accept_hdr_async(socket, |request: &Request, response: Response| {
            if request.uri().path() == expected_path && allowed_webview_origin(request) {
                Ok(response)
            } else {
                Err(ErrorResponse::new(Some("Not found".into())))
            }
        })
        .await
        .map_err(|error| AppError::Generic(format!("VNC WebSocket handshake failed: {error}")))?;
    let remote = TcpStream::connect((host, port))
        .await
        .map_err(|error| AppError::Generic(format!("VNC connection failed: {error}")))?;
    let (mut websocket_write, mut websocket_read) = websocket.split();
    let (mut remote_read, mut remote_write) = remote.into_split();
    let mut remote_buffer = vec![0_u8; 64 * 1024];

    loop {
        tokio::select! {
            message = websocket_read.next() => match message {
                Some(Ok(Message::Binary(bytes))) => remote_write.write_all(&bytes).await?,
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
            matches!(
                origin,
                "tauri://localhost"
                    | "http://tauri.localhost"
                    | "https://tauri.localhost"
                    | "http://localhost:1420"
            )
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

    use super::{start_proxy, viewer_args, vnc_endpoint, VncProxyManager};

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
}
