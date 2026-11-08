use crate::{
    ApiError, RULES_VERSION, Result, blocking,
    bots::BotSpec,
    game::{
        Game, GameEvent, Server, Submission, cancelled_notice, portable_checkpoint,
        wait_for_shutdown,
    },
    history::History,
    presence::Limits,
    replay::{Archive, MAX_ARCHIVE_BYTES, sha256},
    shares::{Metadata, Seek},
};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket, close_code},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use goa2_core::{
    codec::Phase,
    interpreter::live_game::{
        GameCheckpoint, GameContent, GameMode, GameSetup, PlayerCommand, SetupTeam,
        TimeControlConfig,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{broadcast::error::RecvError, watch};
use uuid::Uuid;

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/heroes", get(heroes))
        .route("/heroes/metadata", get(hero_metadata))
        .route("/maps", get(maps))
        .route("/overrides/schema", get(override_schema))
        .route("/games", post(create))
        .route("/__hosted/games/{game}", put(provision))
        .route(
            "/__hosted/games/{game}/report-position",
            post(report_position),
        )
        .route("/__hosted/replays", get(staff_replay_catalog))
        .route("/__hosted/games/{game}/share", post(staff_create_share))
        .route("/__hosted/shares", get(staff_share_catalog))
        .route("/__hosted/shares/{token}", delete(staff_revoke_share))
        .route("/__hosted/games/{game}/replay", get(staff_replay_meta))
        .route(
            "/__hosted/games/{game}/replay/state",
            get(staff_replay_state),
        )
        .route("/games/{game}", get(view))
        .route("/games/{game}/health", get(health))
        .route("/games/{game}/spectator-token", get(spectator))
        .route("/games/{game}/ws", get(websocket))
        .route("/games/{game}/share", post(create_share))
        .route("/shared/{token}", get(shared_metadata))
        .route("/shared/{token}/state", get(shared_state))
        .route("/games/{game}/overrides/history", get(decision_history))
        .route("/games/{game}/overrides/schema", get(override_schema))
        .route("/games/{game}/upgrades/preview", post(upgrade_preview))
        .route("/games/{game}/advance", post(advance))
        .route("/games/{game}/cheats/gold", post(give_gold))
        .route("/games/{game}/{action}", post(action))
        .route("/__lab/health", get(lab_health))
        .route("/__lab/restore", post(lab_restore))
        .route("/__lab/validate-snapshot", post(lab_validate_snapshot))
        .route("/__lab/games/{game}/snapshot", get(lab_snapshot))
        .route("/__lab/games/{game}/replay", get(lab_export_replay))
        .route(
            "/__lab/replay",
            post(lab_restore_replay).layer(DefaultBodyLimit::max(MAX_ARCHIVE_BYTES * 2 + 1024)),
        )
        .route("/__lab/games/{game}", get(lab_inspect).delete(lab_delete))
        .route("/__lab/games/{game}/checkpoint", get(lab_checkpoint))
        .route("/__lab/games/{game}/responders", get(lab_responders))
        .route("/__lab/games/{game}/command", post(lab_command))
        .route("/__lab/games/{game}/comparison", post(lab_comparison))
        .with_state(Arc::clone(&server))
        .layer(middleware::from_fn_with_state(server, no_store))
}

async fn no_store(State(server): State<Arc<Server>>, request: Request, next: Next) -> Response {
    let stopping = *server.shutdown.borrow();
    let mut response = if stopping {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Game service is restarting",
        )
        .into_response()
    } else {
        next.run(request).await
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn health(State(server): State<Arc<Server>>) -> Json<Value> {
    let mut identity = server.identity.clone();
    identity["ok"] = json!(true);
    Json(identity)
}

async fn override_schema() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        include_str!("override_schema.json"),
    )
}

#[derive(Default, Deserialize)]
struct HeroListQuery {
    #[serde(default)]
    include_playtest: bool,
}

#[derive(Deserialize, Serialize)]
struct HeroMetadata {
    id: String,
    difficulty_stars: u8,
}

#[derive(Deserialize)]
struct HeroCatalogue {
    heroes: Vec<String>,
    playtest_heroes: Vec<String>,
    metadata: Vec<HeroMetadata>,
}

impl HeroCatalogue {
    fn get() -> &'static Self {
        static CATALOGUE: OnceLock<HeroCatalogue> = OnceLock::new();
        CATALOGUE.get_or_init(|| {
            serde_json::from_str(include_str!("hero_catalogue.json"))
                .expect("embedded hero catalogue is valid")
        })
    }

    fn lists(&self, content: &GameContent, query: &HeroListQuery, name: &str) -> bool {
        content.has_hero(name)
            && (query.include_playtest || !self.playtest_heroes.iter().any(|hero| hero == name))
    }
}

async fn heroes(
    State(server): State<Arc<Server>>,
    Query(query): Query<HeroListQuery>,
) -> Json<Vec<&'static str>> {
    let catalogue = HeroCatalogue::get();
    Json(
        catalogue
            .heroes
            .iter()
            .map(String::as_str)
            .filter(|name| catalogue.lists(&server.content, &query, name))
            .collect(),
    )
}

async fn hero_metadata(
    State(server): State<Arc<Server>>,
    Query(query): Query<HeroListQuery>,
) -> Json<Vec<&'static HeroMetadata>> {
    let catalogue = HeroCatalogue::get();
    Json(
        catalogue
            .metadata
            .iter()
            .filter(|hero| catalogue.lists(&server.content, &query, &hero.id))
            .collect(),
    )
}

async fn maps(State(server): State<Arc<Server>>) -> Json<Vec<String>> {
    Json(
        server
            .content
            .map_names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
}

#[derive(Deserialize, Serialize)]
struct CreateRequest {
    #[serde(default = "default_map")]
    map_name: String,
    red_heroes: Vec<String>,
    blue_heroes: Vec<String>,
    #[serde(default)]
    game_type: GameMode,
    #[serde(default)]
    cheats_enabled: bool,
    seed: Option<u64>,
    tie_breaker_team: Option<SetupTeam>,
    #[serde(default)]
    player_names: BTreeMap<String, String>,
    time_control: Option<TimeControlConfig>,
    bots: Option<BTreeMap<String, BotSpec>>,
}

fn default_map() -> String {
    "forgotten_island".into()
}

async fn create(
    State(server): State<Arc<Server>>,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    server.require_local_creation()?;
    create_game(server, body, None).await
}

async fn provision(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    if !id.strip_prefix("rust-").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }) || body.seed.is_none()
    {
        return Err(ApiError::invalid(
            "Provisioning requires a Rust game ID and an explicit seed",
        ));
    }
    let hash = sha256(&serde_json::to_vec(&body).map_err(ApiError::internal)?);
    create_game(server, body, Some((id, hash))).await
}

async fn create_game(
    server: Arc<Server>,
    body: CreateRequest,
    provisioning: Option<(String, String)>,
) -> Result<(StatusCode, Json<Value>)> {
    let setup = GameSetup {
        map: body.map_name,
        red_heroes: body.red_heroes,
        blue_heroes: body.blue_heroes,
        seed: body.seed.unwrap_or_else(|| Uuid::new_v4().as_u128() as u64),
        mode: body.game_type,
        cheats_enabled: body.cheats_enabled,
        tie_breaker: body.tie_breaker_team,
    };
    let result = blocking(move || {
        server.create(
            &setup,
            body.player_names,
            body.bots.unwrap_or_default(),
            body.time_control,
            provisioning,
        )
    })
    .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn authenticate(
    server: &Server,
    id: &str,
    headers: &HeaderMap,
) -> Result<(Arc<Game>, Option<String>)> {
    let token = header_value(headers, "authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::unauthorized("Missing bearer token"))?;
    let game = server.game(id)?;
    let viewer = game.viewer(token)?;
    Ok((game, viewer))
}

async fn view(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    Ok(Json(
        blocking(move || {
            let engine = game.snapshot()?;
            game.view(&engine, viewer.as_deref())
        })
        .await?,
    ))
}

async fn replay_worker(server: &Server) -> Result<tokio::sync::OwnedSemaphorePermit> {
    tokio::time::timeout(
        Duration::from_secs(60),
        Arc::clone(&server.share_baker).acquire_owned(),
    )
    .await
    .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Replay baker is busy"))?
    .map_err(ApiError::internal)
}

async fn create_share(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>)> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    if viewer.is_none() {
        return Err(ApiError::forbidden(
            "Spectators cannot create replay shares",
        ));
    }
    let checked_game = game.clone();
    blocking(move || checked_game.require_finished()).await?;
    let permit = replay_worker(&server).await?;
    let token = blocking(move || {
        let _permit = permit;
        game.share(&server.store)
    })
    .await?;
    Ok(share_link(token))
}

fn share_link(token: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::CREATED,
        Json(json!({"token": token, "url": format!("/shared/{token}")})),
    )
}

async fn shared_metadata(
    State(server): State<Arc<Server>>,
    Path(token): Path<String>,
) -> Result<Response> {
    let body = blocking(move || server.store.share_metadata(&token)).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}

async fn shared_state(
    State(server): State<Arc<Server>>,
    Path(token): Path<String>,
    Query(seek): Query<Seek>,
    headers: HeaderMap,
) -> Result<Response> {
    let gzip = header_value(&headers, "accept-encoding").is_some_and(|encodings| {
        encodings
            .split(',')
            .any(|encoding| encoding.trim() == "gzip")
    });
    let body = blocking(move || {
        let packed = server.store.share_group(&token, &seek)?;
        if gzip {
            return Ok(packed);
        }
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(packed.as_slice())
            .read_to_end(&mut json)
            .map_err(ApiError::internal)?;
        Ok(json)
    })
    .await?;
    let mut response = (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::VARY, "Accept-Encoding"),
        ],
        body,
    )
        .into_response();
    if gzip {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    Ok(response)
}

async fn report_position(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    Ok(Json(
        blocking(move || game.report_position(&server.store, viewer.as_deref())).await?,
    ))
}

async fn staff_replay_catalog(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Value>>> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    Ok(Json(blocking(move || server.store.replay_catalog()).await?))
}

async fn staff_create_share(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>)> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    let permit = replay_worker(&server).await?;
    let token = blocking(move || {
        let _permit = permit;
        server.staff_share(&id)
    })
    .await?;
    Ok(share_link(token))
}

async fn staff_share_catalog(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Value>>> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    Ok(Json(blocking(move || server.store.share_catalog()).await?))
}

async fn staff_revoke_share(
    State(server): State<Arc<Server>>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    blocking(move || server.store.revoke_share(&token)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn staff_replay_meta(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Metadata>> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    let permit = replay_worker(&server).await?;
    Ok(Json(
        blocking(move || {
            let _permit = permit;
            Metadata::debug(&id, server.staff_replay(&id)?)
        })
        .await?,
    ))
}

async fn staff_replay_state(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    Query(seek): Query<Seek>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    server.require_hosted(header_value(&headers, "x-game-provision-token"))?;
    let permit = replay_worker(&server).await?;
    Ok(Json(
        blocking(move || {
            let _permit = permit;
            seek.debug(server.staff_replay(&id)?)
        })
        .await?,
    ))
}

async fn decision_history(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<History>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    Ok(Json(
        blocking(move || game.history(&server.store, viewer.as_deref())).await?,
    ))
}

async fn spectator(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    if viewer.is_none() {
        return Err(ApiError::forbidden(
            "Only players can request a spectator link",
        ));
    }
    Ok(Json(game.spectator_link()))
}

#[derive(Deserialize)]
struct UpgradePreviewRequest {
    request_id: String,
    #[serde(default)]
    card_ids: Vec<String>,
}

async fn upgrade_preview(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpgradePreviewRequest>,
) -> Result<Json<Value>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    let viewer = viewer.ok_or_else(|| ApiError::forbidden("Spectators cannot preview upgrades"))?;
    Ok(Json(
        blocking(move || {
            let engine = game.snapshot()?;
            if !engine.request_for(&viewer).is_some_and(|offer| {
                offer["id"] == body.request_id && offer["request_type"] == "UPGRADE_PHASE"
            }) {
                return Err(ApiError::conflict("Refresh the current upgrade offer"));
            }
            Ok(engine.preview_upgrades_at(
                &viewer,
                &body.request_id,
                &body.card_ids,
                game.clock_now(),
            )?)
        })
        .await?,
    ))
}

async fn give_gold(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    let hero = viewer.ok_or_else(|| ApiError::forbidden("Spectators cannot use cheats"))?;
    Ok(Json(
        submit(server, game, hero, "CHEATS_GOLD", body).await?.reply,
    ))
}

async fn advance(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    let hero = viewer.ok_or_else(|| ApiError::forbidden("Spectators cannot advance"))?;
    if game.is_bot(&hero) {
        return Err(ApiError::forbidden("This seat is controlled by a bot"));
    }
    let result = blocking(move || {
        let engine = game.snapshot()?;
        if engine.position().phase == Phase::Planning {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Cannot advance during planning. Commit a card or pass.",
            ));
        }
        Ok(engine.client_boundary_result(Some(&hero))?)
    })
    .await?;
    Ok(Json(result))
}

async fn action(
    State(server): State<Arc<Server>>,
    Path((id, kind)): Path<(String, String)>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Json<Value>> {
    let (game, viewer) = authenticate(&server, &id, &headers)?;
    let hero = viewer.ok_or_else(|| ApiError::forbidden("Spectators cannot submit commands"))?;
    let body = body.map_or_else(|| json!({}), |Json(body)| body);
    let kind = match kind.as_str() {
        "cards" => "COMMIT_CARD",
        "uncommit" => "UNCOMMIT_CARD",
        "pass" => "PASS_TURN",
        "planning-done" => "FINISH_PLANNING",
        "input" => "SUBMIT_INPUT",
        "rollback" => "ROLLBACK",
        "ready" => "SET_READY",
        "starting-position" => "STARTING_POSITION",
        _ => return Err(ApiError::not_found("Unsupported game endpoint")),
    };
    let submission = submit(server, game, hero, kind, body).await?;
    Ok(Json(submission.reply))
}

fn text_field<'a>(body: &'a Value, key: &str) -> Result<&'a str> {
    body[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::invalid(format!("{key} must be a nonempty string")))
}

fn client_action_id(body: &Value) -> Result<Option<String>> {
    let Some(value) = body.get("client_action_id") else {
        return Ok(None);
    };
    let id = value
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        })
        .ok_or_else(|| ApiError::invalid("Invalid client_action_id"))?;
    Ok(Some(id.to_owned()))
}

async fn submit(
    server: Arc<Server>,
    game: Arc<Game>,
    hero: String,
    kind: &str,
    body: Value,
) -> Result<Submission> {
    if game.is_bot(&hero) {
        return Err(ApiError::forbidden("This seat is controlled by a bot"));
    }
    let command = player_command(kind, &body)?;
    let expected = body
        .get("expected_revision")
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| ApiError::invalid("Invalid expected_revision"))
        })
        .transpose()?;
    let id = client_action_id(&body)?;
    blocking(move || game.submit(&server.store, &hero, expected, id, command)).await
}

fn player_command(kind: &str, body: &Value) -> Result<PlayerCommand> {
    if !body.is_object() {
        return Err(ApiError::invalid("Game command must be an object"));
    }
    Ok(match kind {
        "COMMIT_CARD" => PlayerCommand::Commit {
            card_id: text_field(body, "card_id")?.to_owned(),
        },
        "UNCOMMIT_CARD" => PlayerCommand::Uncommit,
        "PASS_TURN" => PlayerCommand::Pass,
        "FINISH_PLANNING" => PlayerCommand::FinishPlanning,
        "CHEATS_GOLD" => PlayerCommand::CheatGold {
            hero_id: text_field(body, "hero_id")?.to_owned(),
            amount: body["amount"]
                .as_i64()
                .ok_or_else(|| ApiError::invalid("Amount must be a positive integer"))?,
        },
        "ROLLBACK" => PlayerCommand::Rollback,
        "SET_READY" => PlayerCommand::Ready {
            ready: body["ready"]
                .as_bool()
                .ok_or_else(|| ApiError::invalid("ready must be a boolean"))?,
        },
        "STARTING_POSITION" => PlayerCommand::StartingPosition {
            operation: serde_json::from_value(body.clone()).map_err(|error| {
                ApiError::invalid(format!("Invalid starting-position command: {error}"))
            })?,
        },
        "SUBMIT_INPUT" => PlayerCommand::Input {
            request_id: text_field(body, "request_id")?.to_owned(),
            selection: body["selection"].clone(),
        },
        _ => return Err(ApiError::invalid("Unsupported game command")),
    })
}

#[derive(Deserialize)]
struct SocketQuery {
    token: String,
}

async fn websocket(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    Query(query): Query<SocketQuery>,
    socket: WebSocketUpgrade,
) -> Result<Response> {
    let game = server.game(&id)?;
    let viewer = game.viewer(&query.token)?;
    let shutdown = server.shutdown.subscribe();
    Ok(socket
        .max_message_size(64 * 1024)
        .on_upgrade(move |socket| socket_game(socket, server, game, viewer, shutdown)))
}

async fn send(socket: &mut WebSocket, message: Value) -> bool {
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(message.to_string().into())),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

async fn send_current(
    socket: &mut WebSocket,
    game: Arc<Game>,
    viewer: Option<String>,
    last_revision: &mut u64,
    action_id: Option<String>,
) -> bool {
    let update = blocking(move || {
        let engine = game.snapshot()?;
        game.state_update(&engine, viewer.as_deref())
    })
    .await;
    let Ok(mut update) = update else { return false };
    *last_revision = update["revision"].as_u64().unwrap_or(*last_revision);
    if let Some(id) = action_id {
        update["client_action_id"] = json!(id);
    }
    send(socket, update).await
}

async fn socket_game(
    mut socket: WebSocket,
    server: Arc<Server>,
    game: Arc<Game>,
    viewer: Option<String>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut updates = game.updates.subscribe();
    let mut presence = game.presence.subscribe();
    let mut presence_limits = Limits::default();
    let mut retired = game.retired.subscribe();
    let Ok(_presence) = game.connect_voter(viewer.as_deref()) else {
        return;
    };
    let mut last_revision = 0;
    let mut seen_override = None;
    if !send_current(
        &mut socket,
        game.clone(),
        viewer.clone(),
        &mut last_revision,
        None,
    )
    .await
    {
        return;
    }
    if !sync_override_notice(&mut socket, &game, &mut seen_override).await {
        return;
    }
    loop {
        tokio::select! {
            _ = wait_for_shutdown(&mut shutdown) => {
                let close = Message::Close(Some(CloseFrame {
                    code: close_code::RESTART,
                    reason: "Game service is restarting".into(),
                }));
                let _ = tokio::time::timeout(Duration::from_secs(2), socket.send(close)).await;
                break;
            }
            _ = retired.changed() => break,
            incoming = socket.recv() => {
                if *shutdown.borrow() {
                    continue;
                }
                let Some(Ok(message)) = incoming else { break };
                let text = match message {
                    Message::Close(_) => break,
                    Message::Text(text) => text,
                    _ => continue,
                };
                let body: Value = match serde_json::from_str(&text) {
                    Ok(body) => body,
                    Err(_) => {
                        if !send(&mut socket, json!({"type": "ERROR", "detail": "Invalid JSON message"})).await { break; }
                        continue;
                    }
                };
                let kind = body["type"].as_str().unwrap_or_default().to_owned();
                if kind == "GET_VIEW" {
                    if !send_current(&mut socket, game.clone(), viewer.clone(), &mut last_revision, None).await { break; }
                    continue;
                }
                if matches!(kind.as_str(), "PING" | "POINTER") {
                    let result = match viewer.as_deref() {
                        Some(actor) => presence_limits.send(&game, actor, &kind, body).await,
                        None => Err(ApiError::forbidden("Spectators cannot send table messages")),
                    };
                    if let Err(error) = result
                        && !send(&mut socket, json!({"type": "ERROR", "detail": error.detail})).await
                    {
                        break;
                    }
                    continue;
                }
                if matches!(kind.as_str(), "PROPOSE_OVERRIDE" | "VOTE_OVERRIDE" | "CANCEL_OVERRIDE") {
                    let result = match viewer.as_ref() {
                        Some(actor) => {
                            let game = game.clone();
                            let server = server.clone();
                            let actor = actor.clone();
                            blocking(move || game.override_command(&server, &actor, &kind, body)).await
                        }
                        None => Err(ApiError::forbidden("Spectators cannot vote")),
                    };
                    match result {
                        Ok(Some((id, deadline))) => game.schedule_vote_expiry(&server, id, deadline),
                        Ok(None) => {}
                        Err(error) => {
                            if !send(&mut socket, json!({"type": "ERROR", "detail": error.detail})).await { break; }
                        }
                    }
                    continue;
                }
                let id = client_action_id(&body).ok().flatten();
                let ready = body["ready"].as_bool();
                let reply = match viewer.as_ref() {
                    Some(hero) => submit(server.clone(), game.clone(), hero.clone(), &kind, body).await,
                    None => Err(ApiError::forbidden("Spectators cannot submit commands")),
                };
                let (mut message, replayed) = match reply {
                    Ok(Submission { mut reply, replayed }) => {
                        reply["type"] = json!(match kind.as_str() {
                            "STARTING_POSITION" => "STARTING_POSITION_UPDATED",
                            "SET_READY" => {
                                reply["hero_id"] = json!(viewer);
                                reply["ready"] = json!(ready);
                                "READY_UPDATED"
                            }
                            _ => "ACTION_RESULT",
                        });
                        (reply, replayed)
                    }
                    Err(error) => (json!({"type": "ERROR", "detail": error.detail}), false),
                };
                if let Some(id) = &id {
                    message["client_action_id"] = json!(id);
                }
                if !send(&mut socket, message).await { break; }
                if replayed && !send_current(&mut socket, game.clone(), viewer.clone(), &mut last_revision, id).await {
                    break;
                }
            }
            received = presence.recv() => match received {
                Ok(notice) if notice.visible_to(viewer.as_deref()) => {
                    if !send(&mut socket, (*notice.body).clone()).await { break; }
                }
                Err(RecvError::Closed) => break,
                _ => {}
            },
            received = updates.recv() => match received {
                Ok(GameEvent::OverrideNotice(notice)) => {
                    if !send_override_notice(&mut socket, (*notice).clone(), &mut seen_override).await { break; }
                }
                Ok(GameEvent::PlayerLinkRevealed(hero)) => {
                    if viewer.as_deref().is_some_and(|viewer| !game.is_bot(viewer))
                        && let Some(message) = game.player_link(&hero)
                        && !send(&mut socket, message).await
                    {
                        break;
                    }
                }
                Ok(GameEvent::State(update)) => {
                    let revision = update.revision.position().revision;
                    if revision <= last_revision {
                        continue;
                    }
                    let (game, viewer) = (game.clone(), viewer.clone());
                    let Ok(message) = blocking(move || update.message(&game, viewer.as_deref())).await else { break };
                    if !send(&mut socket, message).await { break; }
                    last_revision = revision;
                }
                Err(RecvError::Lagged(_)) => {
                    if !send_current(&mut socket, game.clone(), viewer.clone(), &mut last_revision, None).await ||
                       !sync_override_notice(&mut socket, &game, &mut seen_override).await { break; }
                }
                Err(RecvError::Closed) => break,
            }
        }
    }
}

async fn send_override_notice(
    socket: &mut WebSocket,
    notice: Value,
    seen: &mut Option<String>,
) -> bool {
    if notice["type"] == "OVERRIDE_PROPOSED" {
        *seen = notice["proposal_id"].as_str().map(str::to_owned);
    } else if notice["type"] == "OVERRIDE_RESOLVED"
        && seen.as_deref() == notice["proposal_id"].as_str()
    {
        *seen = None;
    }
    send(socket, notice).await
}

async fn sync_override_notice(
    socket: &mut WebSocket,
    game: &Game,
    seen: &mut Option<String>,
) -> bool {
    let Ok(notice) = game.override_notice() else {
        return false;
    };
    if let Some(id) = seen.clone()
        && notice
            .as_ref()
            .and_then(|value| value["proposal_id"].as_str())
            != Some(&id)
        && !send_override_notice(socket, cancelled_notice(&id), seen).await
    {
        return false;
    }
    match notice {
        Some(notice) => send_override_notice(socket, notice, seen).await,
        None => true,
    }
}

fn lab_auth(server: &Server, headers: &HeaderMap) -> Result<()> {
    server.require_lab(header_value(headers, "x-lab-token"))
}

async fn lab_health(State(server): State<Arc<Server>>, headers: HeaderMap) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    Ok(health(State(server)).await)
}

async fn lab_inspect(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    Ok(Json(
        blocking(move || {
            let engine = game.snapshot()?;
            let position = engine.position();
            let decision = engine
                .decision_players()?
                .into_iter()
                .find_map(|id| engine.request_for(id));
            Ok(json!({
                "revision": position.revision,
                "hash": game.lab_revision(position.revision),
                "phase": position.phase,
                "decision": decision.map(|prompt| json!({
                    "type": prompt["request_type"], "player_id": prompt["player_id"]
                })),
            }))
        })
        .await?,
    ))
}

async fn lab_delete(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    blocking(move || server.remove(&id)).await?;
    Ok(Json(json!({"ok": true})))
}

async fn lab_snapshot(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    Ok(Json(blocking(move || game.lab_snapshot()).await?))
}

async fn lab_export_replay(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    Ok(Json(blocking(move || {
        let archive = game.replay(&server.store)?;
        let replay_json = serde_json::to_string(&archive).map_err(ApiError::internal)?;
        if replay_json.len() > MAX_ARCHIVE_BYTES {
            return Err(ApiError::invalid("Live replay exceeds the archive size limit"));
        }
        Ok(json!({"start_revision": archive.start_revision, "revision": archive.revision, "replay_json": replay_json}))
    }).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayRequest {
    replay_json: String,
    revision: Option<u64>,
}

async fn lab_restore_replay(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(request): Json<ReplayRequest>,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    if request.replay_json.len() > MAX_ARCHIVE_BYTES {
        return Err(ApiError::invalid(
            "Live replay exceeds the archive size limit",
        ));
    }
    Ok(Json(
        blocking(move || {
            let archive: Archive = serde_json::from_str(&request.replay_json)
                .map_err(|error| ApiError::invalid(format!("Invalid Rust live replay: {error}")))?;
            let (engine, names) = archive.restore(request.revision)?;
            Ok(json!({
                "checkpoint": portable_checkpoint(&engine, &names)?,
                "revision": engine.position().revision,
            }))
        })
        .await?,
    ))
}

async fn lab_checkpoint(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    Ok(Json(blocking(move || game.checkpoint()).await?))
}

async fn lab_responders(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    Ok(Json(
        blocking(move || {
            let engine = game.snapshot()?;
            let players = match engine.waiting_ready_players() {
                _ if engine.pause_vote_hold().is_some() => Vec::new(),
                Some(unready) => unready
                    .into_iter()
                    .filter(|hero| !game.is_bot(hero))
                    .collect(),
                None => engine.decision_players()?,
            };
            Ok(json!({"revision": engine.position().revision, "players": players}))
        })
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreRequest {
    checkpoint: Option<GameCheckpoint>,
    checkpoint_json: Option<String>,
    #[serde(default)]
    hero_names: BTreeMap<String, String>,
    #[serde(default)]
    bots: BTreeMap<String, BotSpec>,
    #[serde(default)]
    freeze_clock: bool,
}

async fn lab_restore(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(body): Json<RestoreRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    lab_auth(&server, &headers)?;
    let result = blocking(move || {
        let checkpoint = match (body.checkpoint, body.checkpoint_json) {
            (Some(checkpoint), None) => checkpoint,
            (None, Some(text)) => decode_checkpoint(&text)?,
            _ => return Err(ApiError::invalid("Supply exactly one checkpoint")),
        };
        server.restore(checkpoint, body.hero_names, body.bots, body.freeze_clock)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

fn decode_checkpoint(text: &str) -> Result<GameCheckpoint> {
    serde_json::from_str(text)
        .map_err(|error| ApiError::invalid(format!("Invalid Rust checkpoint: {error}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateSnapshot {
    checkpoint_json: String,
}

async fn lab_validate_snapshot(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(body): Json<ValidateSnapshot>,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    blocking(move || Server::restore_engine(decode_checkpoint(&body.checkpoint_json)?)).await?;
    Ok(Json(json!({"ok": true})))
}

async fn lab_command(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    let actor = text_field(&body, "actor")?.to_owned();
    let kind = text_field(&body["message"], "type")?.to_owned();
    let revision = body["expected_revision"]
        .as_u64()
        .ok_or_else(|| ApiError::invalid("Lab commands require expected_revision"))?;
    let mut message = body["message"].clone();
    message["expected_revision"] = json!(revision);
    Ok(Json(
        submit(server, game, actor, &kind, message).await?.reply,
    ))
}

async fn lab_comparison(
    State(server): State<Arc<Server>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    lab_auth(&server, &headers)?;
    let game = server.game(&id)?;
    let action = if let Some(message) = body.get("message") {
        let actor = text_field(&body, "actor")?.to_owned();
        let expected = body["expected_revision"]
            .as_u64()
            .ok_or_else(|| ApiError::invalid("Lab previews require expected_revision"))?;
        let command = player_command(text_field(message, "type")?, message)?;
        Some((actor, expected, command, client_action_id(message)?))
    } else {
        None
    };
    Ok(Json(
        blocking(move || {
            let action = action
                .map(|(actor, expected, command, action_id)| {
                    if let Some(action_id) = action_id {
                        let value = serde_json::to_value(&command).map_err(ApiError::internal)?;
                        if server
                            .store
                            .accepted(&id, &actor, &action_id, &value)?
                            .is_some()
                        {
                            return Err(ApiError::conflict("This action was already accepted"));
                        }
                    }
                    Ok((actor, expected, command))
                })
                .transpose()?;
            let (revision, views) = game.lab_comparison(action)?;
            Ok(json!({"revision": revision, "rules_version": RULES_VERSION, "views": views}))
        })
        .await?,
    ))
}
