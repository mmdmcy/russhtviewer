use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::net::TcpListener;

use crate::{
    auth,
    fleet::{
        FleetRegistry, HeartbeatRequest, RegisterRequest, first_tailscale_ipv4, local_identity,
    },
    pty::PtySession,
};

const INDEX_HTML: &str = include_str!("../assets/index.html");

#[derive(Clone)]
pub struct AppState {
    pub token_hash: Option<String>,
    pub port: u16,
    pub role: Role,
    pub fleet: Option<Arc<FleetRegistry>>,
    pub device_id: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum Role {
    Host,
    Agent,
}

pub fn spawn_server(state: AppState) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(err) => {
                tracing::error!(error = %err, "failed to build server runtime");
                return;
            }
        };
        if let Err(err) = runtime.block_on(run_server(state)) {
            tracing::error!(error = %err, "server stopped");
        }
    });
}

async fn run_server(state: AppState) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/v1/device", get(device_info))
        .route("/v1/fleet/register", post(fleet_register))
        .route("/v1/fleet/heartbeat", post(fleet_heartbeat))
        .route("/v1/fleet/devices", get(fleet_devices))
        .route("/v1/pty", get(pty_ws))
        .with_state(Arc::new(state.clone()));

    let mut servers = tokio::task::JoinSet::new();
    let loopback = SocketAddr::from(([127, 0, 0, 1], state.port));
    let listener = TcpListener::bind(loopback)
        .await
        .with_context(|| format!("bind {loopback}"))?;
    tracing::info!("listening on {loopback}");
    spawn_serve(&mut servers, listener, app.clone());

    if let Some(ip) = first_tailscale_ipv4().and_then(|ip| ip.parse::<Ipv4Addr>().ok()) {
        let addr = SocketAddr::new(IpAddr::V4(ip), state.port);
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                tracing::info!("listening on {addr} (tailscale)");
                spawn_serve(&mut servers, listener, app.clone());
            }
            Err(err) => tracing::warn!(error = %err, "tailscale bind failed"),
        }
    }

    while let Some(joined) = servers.join_next().await {
        if let Err(err) = joined.context("listener task")? {
            tracing::warn!(error = %err, "listener exited");
        }
    }
    Ok(())
}

fn spawn_serve(
    servers: &mut tokio::task::JoinSet<Result<()>>,
    listener: TcpListener,
    app: Router,
) {
    servers.spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .context("serve")
    });
}

async fn index() -> Response {
    Response::builder()
        .header(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )
        .body(Body::from(INDEX_HTML))
        .unwrap()
}

async fn device_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let identity = local_identity(state.port, state.device_id.as_deref());
    Json(json!({
        "device_id": identity.device_id,
        "display_name": identity.display_name,
        "os": identity.os,
        "role": match state.role {
            Role::Host => "host",
            Role::Agent => "agent",
        },
        "fleet_enabled": state.fleet.is_some(),
        "shell_url": identity.shell_url,
    }))
}

async fn fleet_register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize(&headers, None, &state)?;
    let registry = state.fleet.clone().ok_or((
        StatusCode::NOT_FOUND,
        "fleet registry only available on host".into(),
    ))?;
    Ok(Json(registry.register(request)))
}

async fn fleet_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<HeartbeatRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize(&headers, None, &state)?;
    let registry = state.fleet.clone().ok_or((
        StatusCode::NOT_FOUND,
        "fleet registry only available on host".into(),
    ))?;
    registry
        .heartbeat(request)
        .map(Json)
        .map_err(|err| (StatusCode::NOT_FOUND, err.to_string()))
}

async fn fleet_devices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize(&headers, None, &state)?;
    let registry = state.fleet.clone().ok_or((
        StatusCode::NOT_FOUND,
        "fleet registry only available on host".into(),
    ))?;
    Ok(Json(json!({ "devices": registry.list() })))
}

#[derive(Debug, Deserialize)]
struct PtyQuery {
    access_token: Option<String>,
}

async fn pty_ws(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PtyQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize(&headers, query.access_token.as_deref(), &state)?;
    Ok(ws.on_upgrade(handle_pty_socket))
}

async fn handle_pty_socket(socket: WebSocket) {
    let handles = match PtySession::spawn(120, 40) {
        Ok(handles) => handles,
        Err(err) => {
            tracing::error!(error = %err, "failed to spawn pty");
            return;
        }
    };
    let session = handles.session;
    let mut output_rx = handles.output_rx;

    let (mut sink, mut stream) = socket.split();
    let session_out = session.clone();
    let pump = tokio::spawn(async move {
        loop {
            if let Some(code) = session_out.try_wait() {
                let _ = sink
                    .send(Message::Text(
                        json!({"type":"exit","code":code}).to_string().into(),
                    ))
                    .await;
                break;
            }
            tokio::select! {
                maybe = output_rx.recv() => {
                    match maybe {
                        Some(bytes) => {
                            let data = String::from_utf8_lossy(&bytes).into_owned();
                            if sink
                                .send(Message::Text(
                                    json!({"type":"output","data":data}).to_string().into(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        match message {
            Message::Text(text) => {
                if let Ok(msg) = serde_json::from_str::<PtyClientMessage>(&text) {
                    match msg {
                        PtyClientMessage::Input { data } => {
                            let _ = session.write_input(data.as_bytes());
                        }
                        PtyClientMessage::Resize { cols, rows } => {
                            let _ = session.resize(cols, rows);
                        }
                    }
                }
            }
            Message::Binary(bytes) => {
                let _ = session.write_input(&bytes);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    pump.abort();
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PtyClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

fn authorize(
    headers: &HeaderMap,
    query_token: Option<&str>,
    state: &AppState,
) -> Result<(), (StatusCode, String)> {
    let Some(expected) = state.token_hash.as_deref() else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "DALE_TOKEN / RUSSH_MASTERDALE_TOKEN is not configured on this node".into(),
        ));
    };
    let token = bearer_token(headers)
        .or_else(|| query_token.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string));
    let Some(token) = token else {
        return Err((StatusCode::UNAUTHORIZED, "bearer token required".into()));
    };
    if !auth::token_matches(expected, &token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid token".into()));
    }
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}
