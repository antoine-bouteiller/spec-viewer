mod registry;

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::{OriginalUri, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use futures_util::stream;
use rust_embed::RustEmbed;

use crate::core::{DocError, Viewer, docs};
use crate::document::Documents;

#[derive(RustEmbed)]
#[folder = "$OUT_DIR/assets/"]
struct Assets;

struct App {
    viewer: Viewer,
    documents: Documents,
}

fn text(status: StatusCode, body: &'static str) -> Response {
    (status, [("content-type", "text/plain;charset=utf-8")], body).into_response()
}

fn json(value: impl serde::Serialize) -> Response {
    ([("content-type", "application/json;charset=utf-8")], Json(value)).into_response()
}

fn asset(path: &str, content_type: &'static str, headers: &HeaderMap) -> Response {
    let Some(content) = Assets::get(path) else {
        return text(StatusCode::NOT_FOUND, "not found");
    };
    if path.starts_with("mermaid/") {
        return ([("content-type", content_type)], content.data.into_owned()).into_response();
    }
    let hash: String = content.metadata.sha256_hash().iter().map(|byte| format!("{byte:02x}")).collect();
    let etag = format!("\"{hash}\"");
    let mut response = if headers
        .get("if-none-match")
        .is_some_and(|value| value.as_bytes() == etag.as_bytes())
    {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        content.data.into_owned().into_response()
    };
    let cache = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    response.headers_mut().insert("content-type", content_type.parse().unwrap());
    response.headers_mut().insert("cache-control", cache.parse().unwrap());
    response.headers_mut().insert("etag", etag.parse().unwrap());
    response
}

async fn head_response(request: Request, next: Next) -> Response {
    let head = request.method() == Method::HEAD;
    let mut response = next.run(request).await;
    if head
        && response
            .headers()
            .get("content-type")
            .is_some_and(|value| value == "text/plain;charset=utf-8")
    {
        response.headers_mut().remove("content-type");
    }
    response
}

async fn request(
    State(app): State<Arc<App>>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<Vec<(String, String)>>,
) -> Response {
    let parameter = |name: &str| query.iter().find(|(key, _)| key == name).map_or("", |(_, value)| value.as_str());
    let path = uri.path();
    if path == "/api/projects" {
        return json(serde_json::json!({ "projects": [app.viewer.current()] }));
    }
    if matches!(path, "/api/docs" | "/api/doc" | "/api/events") {
        let Some(worktree) = app.viewer.worktree_at(parameter("wt")) else {
            return text(StatusCode::FORBIDDEN, "forbidden");
        };
        match path {
            "/api/docs" => return json(docs(&worktree, &app.viewer.project.name)),
            "/api/doc" => {
                return match app.documents.open(&worktree, parameter("path")) {
                    Ok(document) => {
                        let rendered = document.body.html();
                        json(serde_json::json!({
                            "degraded": rendered.degraded,
                            "hasMermaid": rendered.has_mermaid,
                            "html": rendered.html,
                            "meta": document.meta,
                            "path": document.path,
                            "rootPath": document.root_path,
                            "toc": rendered.toc,
                        }))
                    }
                    Err(DocError::Forbidden) => text(StatusCode::FORBIDDEN, "forbidden"),
                    Err(DocError::NotFound) => text(StatusCode::NOT_FOUND, "not found"),
                };
            }
            _ => {
                let watch = crate::watch::watch(&worktree).ok();
                let events = stream::unfold(watch, |watch| async move {
                    loop {
                        if watch.as_ref().is_some_and(|watch| watch.reloads.try_recv().is_ok()) {
                            return Some((Ok::<_, Infallible>(Event::default().data("reload")), watch));
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                });
                let mut response = Sse::new(events).into_response();
                response.headers_mut().insert("content-type", "text/event-stream".parse().unwrap());
                return response;
            }
        }
    }
    if method == Method::GET
        && let Some(relative) = path.strip_prefix("/assets/mermaid/")
    {
        let mut normalized = Vec::new();
        for component in relative.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    if normalized.pop().is_none() {
                        return text(StatusCode::FORBIDDEN, "forbidden");
                    }
                }
                component => normalized.push(component),
            }
        }
        if normalized.is_empty() {
            return text(StatusCode::FORBIDDEN, "forbidden");
        }
        if !relative.ends_with(".mjs") && !relative.ends_with(".js") {
            return text(StatusCode::NOT_FOUND, "not found");
        }
        return asset(
            &format!("mermaid/{}", normalized.join("/")),
            "text/javascript; charset=utf-8",
            &headers,
        );
    }
    if path == "/" {
        return asset("index.html", "text/html;charset=utf-8", &headers);
    }
    let relative = path.strip_prefix('/').unwrap_or(path);
    if !relative.starts_with("mermaid/") {
        let content_type = if relative.ends_with(".js") {
            "text/javascript;charset=utf-8"
        } else if relative.ends_with(".css") {
            "text/css;charset=utf-8"
        } else {
            return text(StatusCode::NOT_FOUND, "not found");
        };
        return asset(relative, content_type, &headers);
    }
    text(StatusCode::NOT_FOUND, "not found")
}

pub fn run(root: &Path, port: u16) -> ExitCode {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(start(root, port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("plan-viewer: {error}");
            ExitCode::from(1)
        }
    }
}

async fn start(root: &Path, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let viewer = Viewer::new(root)?;
    let cert = std::env::var("PLAN_VIEWER_CERT").ok().filter(|value| !value.is_empty());
    let key = std::env::var("PLAN_VIEWER_KEY").ok().filter(|value| !value.is_empty());
    if cert.is_some() != key.is_some() {
        return Err("PLAN_VIEWER_CERT and PLAN_VIEWER_KEY must be set together".into());
    }
    let tls = match (cert, key) {
        (Some(cert), Some(key)) => {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            Some(axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?)
        }
        _ => None,
    };
    let bind = std::env::var("PLAN_VIEWER_BIND")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".into());
    let address = bind.parse::<IpAddr>().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let listener = match std::net::TcpListener::bind(SocketAddr::new(address, port)) {
        Ok(listener) => listener,
        Err(error) if port != 0 && error.kind() == std::io::ErrorKind::AddrInUse => {
            std::net::TcpListener::bind(SocketAddr::new(address, 0))?
        }
        Err(error) => return Err(error.into()),
    };
    listener.set_nonblocking(true)?;
    let bound = listener.local_addr()?;
    let scheme = if tls.is_some() { "https" } else { "http" };
    let host = match bound.ip() {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) if address.to_ipv4_mapped().is_some() => {
            let segments = address.segments();
            format!("[::ffff:{:x}:{:x}]", segments[6], segments[7])
        }
        IpAddr::V6(address) => format!("[{address}]"),
    };
    let origin = if (scheme == "http" && bound.port() == 80) || (scheme == "https" && bound.port() == 443) {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{}", bound.port())
    };
    let handle = axum_server::Handle::new();
    if std::env::var("PLAN_VIEWER_MANAGED").as_deref() == Ok("1") {
        let interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::spawn(managed(
            handle.clone(),
            registry::config_path(),
            viewer.project.key.to_string_lossy().into_owned(),
            interrupt,
            terminate,
        ));
    }
    let router = Router::new()
        .fallback(request)
        .layer(middleware::from_fn(head_response))
        .with_state(Arc::new(App {
            viewer,
            documents: Documents::default(),
        }));
    println!("plan-viewer listening on {origin}");
    match tls {
        Some(config) => {
            axum_server::from_tcp_rustls(listener, config)?
                .handle(handle)
                .serve(router.into_make_service())
                .await?
        }
        None => {
            axum_server::from_tcp(listener)?
                .handle(handle)
                .serve(router.into_make_service())
                .await?
        }
    }
    Ok(())
}

fn clear_owned_run(path: &Path, key: &str) -> std::io::Result<()> {
    registry::update_entry(path, key, |entry| match entry {
        Some(entry) if entry.owned() => Some(registry::Entry::stopped(entry.port)),
        entry => entry,
    })?;
    Ok(())
}

fn should_stop_for_empty_run(path: &Path, key: &str) -> std::io::Result<bool> {
    let mut should_stop = true;
    registry::update_entry(path, key, |entry| match entry {
        Some(entry) if entry.owned() && !entry.holders.is_empty() => {
            should_stop = false;
            Some(entry)
        }
        Some(entry) if entry.owned() => Some(registry::Entry::stopped(entry.port)),
        entry => entry,
    })?;
    Ok(should_stop)
}

fn tick(path: &Path, key: &str) -> std::io::Result<bool> {
    let entry = registry::read_entry(path, key);
    let Ok(Some(entry)) = entry else {
        let _ = clear_owned_run(path, key);
        return Ok(true);
    };
    if !entry.owned() {
        let _ = clear_owned_run(path, key);
        return Ok(true);
    }
    if entry.holders.is_empty() {
        return should_stop_for_empty_run(path, key);
    }
    Ok(false)
}

async fn managed(
    handle: axum_server::Handle<SocketAddr>,
    path: std::path::PathBuf,
    key: String,
    mut interrupt: tokio::signal::unix::Signal,
    mut terminate: tokio::signal::unix::Signal,
) {
    enum Operation {
        Tick,
        Stop,
    }
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let stopping = Arc::new(AtomicBool::new(false));
    let producer_stopping = stopping.clone();
    let producer = tokio::spawn(async move {
        let period = Duration::from_secs(5);
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            tokio::select! {
                _ = interval.tick() => { if sender.send(Operation::Tick).is_err() { break; } }
                _ = interrupt.recv() => {
                    producer_stopping.store(true, Ordering::SeqCst);
                    let _ = sender.send(Operation::Stop);
                    break;
                }
                _ = terminate.recv() => {
                    producer_stopping.store(true, Ordering::SeqCst);
                    let _ = sender.send(Operation::Stop);
                    break;
                }
            }
        }
    });
    while let Some(operation) = receiver.recv().await {
        if matches!(operation, Operation::Tick) && stopping.load(Ordering::SeqCst) {
            continue;
        }
        let path = path.clone();
        let key = key.clone();
        let stop = tokio::task::spawn_blocking(move || match operation {
            Operation::Tick => tick(&path, &key),
            Operation::Stop => {
                let _ = clear_owned_run(&path, &key);
                Ok(true)
            }
        })
        .await;
        if !matches!(stop, Ok(Ok(false))) {
            producer.abort();
            handle.shutdown();
            return;
        }
    }
}
