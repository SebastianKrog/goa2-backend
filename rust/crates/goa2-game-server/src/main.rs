#![forbid(unsafe_code)]
mod bots;
mod game;
mod history;
mod http;
mod lab_port;
mod presence;
mod replay;
mod shares;
mod store;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use game::Server;
use goa2_core::interpreter::{EngineError, live_game::GameContent};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env,
    fmt::{self, Display},
    fs::File,
    future::IntoFuture,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use store::Store;

const RULES_VERSION: u32 = 9;

type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    detail: String,
}

impl ApiError {
    fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
        }
    }

    fn invalid(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
    }

    fn conflict(detail: &str) -> Self {
        Self::new(StatusCode::CONFLICT, detail)
    }

    fn unauthorized(detail: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, detail)
    }

    fn forbidden(detail: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, detail)
    }

    fn not_found(detail: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, detail)
    }

    fn internal(error: impl Display) -> Self {
        eprintln!("goa2-game-server: {error}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Game service error")
    }
}

impl Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.status, self.detail)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"detail": self.detail}))).into_response()
    }
}

impl From<EngineError> for ApiError {
    fn from(error: EngineError) -> Self {
        Self::invalid(error.to_string())
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(ApiError::internal)?
}

fn load_content(
    path: &Path,
) -> std::result::Result<(GameContent, Value), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let content = GameContent::read(bytes.as_slice())?;
    let mut executable = if cfg!(target_os = "linux") {
        // Hash the running inode even if the release symlink has been changed.
        File::open("/proc/self/exe")?
    } else {
        File::open(env::current_exe()?)?
    };
    let mut binary_hash = Sha256::new();
    std::io::copy(&mut executable, &mut binary_hash)?;
    let identity = json!({
        "engine": "rust",
        "server_version": env!("CARGO_PKG_VERSION"),
        "rules_version": RULES_VERSION,
        "target_arch": env::consts::ARCH,
        "target_os": env::consts::OS,
        "binary_sha256": format!("{:x}", binary_hash.finalize()),
        "content_sha256": replay::sha256(&bytes),
        "content": content.identity(),
    });
    Ok((content, identity))
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let require_hosted = match args.as_slice() {
        [] => false,
        [mode] if mode == "--hosted" => true,
        [mode] if mode == "--port-lab-situations" => return lab_port::run(),
        [mode, path] if mode == "--backup" => {
            let source = env::var_os("GOA2_GAME_DATABASE").ok_or("Set GOA2_GAME_DATABASE")?;
            Store::backup(Path::new(&source), Path::new(path))?;
            return Ok(());
        }
        [mode, path] if mode == "--check-content" => {
            let (_, identity) = load_content(Path::new(path))?;
            println!("{identity}");
            return Ok(());
        }
        _ => {
            return Err(
                "Use --hosted, --check-content PATH, --backup PATH, --port-lab-situations, or no arguments".into(),
            );
        }
    };
    let pack = PathBuf::from(env::var_os("GOA2_GAME_CONTENT").ok_or("Set GOA2_GAME_CONTENT")?);
    let database =
        PathBuf::from(env::var_os("GOA2_GAME_DATABASE").ok_or("Set GOA2_GAME_DATABASE")?);
    let bind: SocketAddr = env::var("GOA2_GAME_BIND")
        .unwrap_or_else(|_| "127.0.0.1:3288".into())
        .parse()?;
    let (content, identity) = load_content(&pack)?;
    let lab_token = env::var("GOA2_LAB_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    let hosted_token = env::var("GOA2_GAME_PROVISION_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    if require_hosted && hosted_token.is_none() {
        return Err("Hosted mode requires GOA2_GAME_PROVISION_TOKEN".into());
    }
    if lab_token.is_some() && hosted_token.is_some() {
        return Err("Use separate Rust services for hosted games and the Lab".into());
    }
    let server = Arc::new(Server::new(
        content,
        identity,
        Store::open(&database)?,
        lab_token,
        hosted_token,
    )?);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let address = listener.local_addr()?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let signal = async {
        #[cfg(unix)]
        {
            tokio::select! {
                result = tokio::signal::ctrl_c() => result,
                _ = terminate.recv() => Ok(()),
            }
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await
    };
    server.resume_workers()?;
    let (stop_listener, stopped) = tokio::sync::oneshot::channel();
    let serving = axum::serve(listener, http::router(Arc::clone(&server)))
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(serving);
    eprintln!("goa2-game-server listening on {}", address);
    tokio::select! {
        result = &mut serving => result?,
        result = signal => {
            result?;
            server.shutdown.send_replace(true);
            let _ = stop_listener.send(());
            tokio::time::timeout(Duration::from_secs(10), async {
                serving.await?;
                // Upgraded WebSockets run outside Axum's HTTP drain. Their
                // receivers are registered before the upgrade is accepted.
                server.shutdown.closed().await;
                Ok::<_, std::io::Error>(())
            }).await??;
        }
    }
    Ok(())
}
