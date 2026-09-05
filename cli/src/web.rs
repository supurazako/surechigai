use crate::{
    config::Config,
    game::{Deck, MAX_NAME_BYTES, Slot},
    state::State,
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::State as AxumState,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

const INDEX_HTML: &str = include_str!("../../web/index.html");
const APP_JS: &str = include_str!("../../web/app.js");
const STYLE_CSS: &str = include_str!("../../web/style.css");

type SharedState = Arc<Mutex<State>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetupRequest {
    pub name: String,
    pub when: String,
    pub r#where: String,
    pub who: String,
    pub what: String,
    pub why: String,
    pub how: String,
}

impl SetupRequest {
    pub fn from_config(config: &Config) -> Self {
        Self {
            name: config.name.clone(),
            when: config.when.clone(),
            r#where: config.r#where.clone(),
            who: config.who.clone(),
            what: config.what.clone(),
            why: config.why.clone(),
            how: config.how.clone(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.is_empty(), "ユーザー名は空にできません");
        ensure!(
            self.name.len() <= MAX_NAME_BYTES,
            "ユーザー名はUTF-8で{MAX_NAME_BYTES}バイト以内にしてください"
        );
        self.deck()?;
        Ok(())
    }

    pub fn apply_to(self, config: &mut Config) {
        config.name = self.name;
        config.when = self.when;
        config.r#where = self.r#where;
        config.who = self.who;
        config.what = self.what;
        config.why = self.why;
        config.how = self.how;
    }

    fn deck(&self) -> Result<Deck> {
        Deck::new([
            self.when.clone(),
            self.r#where.clone(),
            self.who.clone(),
            self.what.clone(),
            self.why.clone(),
            self.how.clone(),
        ])
    }
}

struct ViewerMeta {
    phase: &'static str,
    role: Option<String>,
    last_error: Option<String>,
    setup: SetupRequest,
    state: Option<SharedState>,
    setup_sender: Option<oneshot::Sender<SetupRequest>>,
}

#[derive(Clone)]
pub struct ViewerHandle {
    inner: Arc<Mutex<ViewerMeta>>,
}

impl ViewerHandle {
    pub fn attach_state(&self, state: SharedState) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = Some(state);
        inner.phase = "running";
        inner.last_error = None;
    }

    pub fn set_role(&self, role: impl Into<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.role = Some(role.into());
        inner.last_error = None;
    }

    pub fn set_error(&self, error: impl Into<String>) {
        self.inner.lock().unwrap().last_error = Some(error.into());
    }

    pub fn set_stopped(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.phase = "stopped";
        inner.role = None;
    }
}

pub struct WebServer {
    viewer: ViewerHandle,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<()>>,
    address: String,
}

impl WebServer {
    pub fn viewer(&self) -> ViewerHandle {
        self.viewer.clone()
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await.context("Web UIタスクの終了に失敗")??;
        Ok(())
    }
}

pub async fn start(config: &Config) -> Result<(WebServer, oneshot::Receiver<SetupRequest>)> {
    let (viewer, setup_receiver) = viewer(config);
    let router = router(viewer.clone());
    let listener = TcpListener::bind(("127.0.0.1", config.web_port))
        .await
        .with_context(|| format!("Web UIのポート{}を使用できません", config.web_port))?;
    let address = format!("http://{}", listener.local_addr()?);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_receiver.await;
            })
            .await
            .context("Web UIサーバーが停止しました")
    });
    Ok((
        WebServer {
            viewer,
            shutdown: Some(shutdown_sender),
            task,
            address,
        },
        setup_receiver,
    ))
}

fn viewer(config: &Config) -> (ViewerHandle, oneshot::Receiver<SetupRequest>) {
    let (setup_sender, setup_receiver) = oneshot::channel();
    let viewer = ViewerHandle {
        inner: Arc::new(Mutex::new(ViewerMeta {
            phase: "setup",
            role: None,
            last_error: None,
            setup: SetupRequest::from_config(config),
            state: None,
            setup_sender: Some(setup_sender),
        })),
    };
    (viewer, setup_receiver)
}

fn router(viewer: ViewerHandle) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(javascript))
        .route("/style.css", get(stylesheet))
        .route("/api/state", get(api_state))
        .route("/api/start", post(api_start))
        .route("/api/generate", post(api_generate))
        .with_state(viewer)
}

async fn index() -> Response {
    asset("text/html; charset=utf-8", INDEX_HTML)
}

async fn javascript() -> Response {
    asset("text/javascript; charset=utf-8", APP_JS)
}

async fn stylesheet() -> Response {
    asset("text/css; charset=utf-8", STYLE_CSS)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

async fn api_start(
    AxumState(viewer): AxumState<ViewerHandle>,
    Json(setup): Json<SetupRequest>,
) -> Result<Json<SetupAccepted>, (StatusCode, Json<ApiError>)> {
    setup.validate().map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: error.to_string(),
            }),
        )
    })?;
    let sender = {
        let mut inner = viewer.inner.lock().unwrap();
        let Some(sender) = inner.setup_sender.take() else {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiError {
                    error: "交換はすでに開始されています".into(),
                }),
            ));
        };
        inner.setup = setup.clone();
        inner.phase = "starting";
        sender
    };
    sender.send(setup).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: "CLIへ設定を渡せませんでした".into(),
            }),
        )
    })?;
    Ok(Json(SetupAccepted { accepted: true }))
}

#[derive(Serialize)]
struct SetupAccepted {
    accepted: bool,
}

#[derive(Deserialize)]
struct GenerateRequest {
    sentence: String,
}

async fn api_generate(
    AxumState(viewer): AxumState<ViewerHandle>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<SetupAccepted>, (StatusCode, Json<ApiError>)> {
    let state = {
        let inner = viewer.inner.lock().unwrap();
        inner.state.clone().ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                Json(ApiError {
                    error: "交換を開始してから画像を生成してください".into(),
                }),
            )
        })?
    };
    state
        .lock()
        .unwrap()
        .request_image(&request.sentence)
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: error.to_string(),
                }),
            )
        })?;
    Ok(Json(SetupAccepted { accepted: true }))
}

async fn api_state(AxumState(viewer): AxumState<ViewerHandle>) -> Json<ViewerResponse> {
    Json(snapshot(&viewer))
}

#[derive(Serialize)]
struct ViewerResponse {
    phase: &'static str,
    role: Option<String>,
    last_error: Option<String>,
    setup: SetupRequest,
    device: Option<DeviceView>,
}

#[derive(Serialize)]
struct DeviceView {
    node: String,
    name: String,
    round: String,
    rendered: String,
    complete: bool,
    missing_count: u32,
    slots: Vec<SlotView>,
    exchanges: Vec<ExchangeView>,
    image_generation_enabled: bool,
    image_generation_busy: bool,
    image_status: Option<String>,
    image_url: Option<String>,
}

#[derive(Serialize)]
struct SlotView {
    key: &'static str,
    label: &'static str,
    text: Option<String>,
    source_name: Option<String>,
    source_node: Option<String>,
}

#[derive(Serialize)]
struct ExchangeView {
    sequence: u64,
    peer_name: String,
    peer_node: String,
    sent: String,
    received: String,
}

fn snapshot(viewer: &ViewerHandle) -> ViewerResponse {
    let (phase, role, last_error, setup, state) = {
        let inner = viewer.inner.lock().unwrap();
        (
            inner.phase,
            inner.role.clone(),
            inner.last_error.clone(),
            inner.setup.clone(),
            inner.state.clone(),
        )
    };
    let device = state.map(|state| {
        let state = state.lock().unwrap();
        let profile = state.profile();
        let sentence = state.sentence();
        let image_status = state.image_status();
        DeviceView {
            node: state.node().to_string(),
            name: profile.name,
            round: sentence.round.to_string(),
            rendered: sentence.render(),
            complete: sentence.is_complete(),
            missing_count: sentence.missing_mask().count_ones(),
            slots: Slot::ALL
                .into_iter()
                .map(|slot| {
                    let entry = sentence.entry(slot);
                    SlotView {
                        key: slot_key(slot),
                        label: slot.label(),
                        text: entry.map(|entry| entry.text.clone()),
                        source_name: entry.map(|entry| entry.source_name.clone()),
                        source_node: entry.map(|entry| entry.source.to_string()),
                    }
                })
                .collect(),
            exchanges: state
                .exchanges()
                .iter()
                .map(|exchange| ExchangeView {
                    sequence: exchange.sequence,
                    peer_name: exchange.peer_name.clone(),
                    peer_node: exchange.peer_node.to_string(),
                    sent: phrase_label(exchange.sent.as_ref()),
                    received: phrase_label(exchange.received.as_ref()),
                })
                .collect(),
            image_generation_enabled: state.image_generation_enabled(),
            image_generation_busy: state.image_generation_busy(),
            image_status: image_status.as_ref().map(|s| s.status.clone()),
            image_url: image_status.and_then(|s| s.image_url),
        }
    });
    ViewerResponse {
        phase,
        role,
        last_error,
        setup,
        device,
    }
}

fn slot_key(slot: Slot) -> &'static str {
    match slot {
        Slot::When => "when",
        Slot::Where => "where",
        Slot::Who => "who",
        Slot::What => "what",
        Slot::Why => "why",
        Slot::How => "how",
    }
}

fn phrase_label(phrase: Option<&crate::game::Phrase>) -> String {
    phrase.map_or_else(
        || "なし".into(),
        |phrase| format!("{}: {}", phrase.slot.label(), phrase.text),
    )
}

pub async fn wait_for_setup(
    receiver: &mut oneshot::Receiver<SetupRequest>,
) -> Result<SetupRequest> {
    receiver.await.context("Web UIが開始前に終了しました")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        game::{ALL_MISSING, Phrase},
        protocol::{GiftPacket, Profile},
    };
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use clap::Parser;
    use std::time::{Duration, Instant};
    use tower::ServiceExt;
    use uuid::Uuid;

    #[test]
    fn setup_uses_cli_defaults_and_validates_limits() {
        let config = Config::try_parse_from(["test", "--web"]).unwrap();
        let mut setup = SetupRequest::from_config(&config);
        assert!(setup.validate().is_ok());
        setup.name = "あ".repeat(11);
        assert!(setup.validate().is_err());
        setup.name = "alice".into();
        setup.who.clear();
        assert!(setup.validate().is_err());
    }

    #[tokio::test]
    async fn api_serves_defaults_rejects_invalid_and_accepts_valid_setup() {
        let config = Config::try_parse_from(["test", "--web"]).unwrap();
        let (viewer, setup_receiver) = viewer(&config);
        let app = router(viewer);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(state["phase"], "setup");
        assert_eq!(state["setup"]["name"], "anonymous");

        let mut invalid = SetupRequest::from_config(&config);
        invalid.who.clear();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&invalid).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut valid = SetupRequest::from_config(&config);
        valid.name = "alice".into();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/start")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&valid).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(setup_receiver.await.unwrap().name, "alice");
    }

    #[tokio::test]
    async fn api_generate_requires_running_state_and_post_url() {
        let config = Config::try_parse_from(["test", "--web"]).unwrap();
        let (viewer, _setup_receiver) = viewer(&config);
        let app = router(viewer.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/generate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"sentence":"ある日、犬が歩いた"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let state = State::new(
            Uuid::new_v4(),
            "alice".into(),
            config.deck().unwrap(),
            Duration::from_secs(5),
            Duration::from_secs(30),
        );
        let state = Arc::new(Mutex::new(state));
        viewer.attach_state(state.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/generate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"sentence":"ある日、犬が歩いた"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        state
            .lock()
            .unwrap()
            .set_post_url(Some("http://127.0.0.1:9/submit".into()));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/generate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"sentence":"ある日、犬が歩いた"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/generate")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"sentence":"続けて生成"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn snapshot_contains_received_source_and_exchange_history() {
        let config = Config::try_parse_from(["test", "--web"]).unwrap();
        let (viewer, _setup_receiver) = viewer(&config);
        let mut state = State::new(
            Uuid::new_v4(),
            "alice".into(),
            config.deck().unwrap(),
            Duration::from_secs(5),
            Duration::from_secs(30),
        );
        let peer = Profile {
            node: Uuid::new_v4(),
            name: "bob".into(),
            round: Uuid::new_v4(),
            missing: ALL_MISSING,
        };
        let sent = state.choose_gift(&peer);
        let received = GiftPacket {
            receiver_round: state.sentence().round,
            gift: Some(Phrase::new(Slot::Who, "猫が".into()).unwrap()),
        };
        state
            .record_exchange(&peer, &sent, &received, Instant::now())
            .unwrap();
        viewer.attach_state(Arc::new(Mutex::new(state)));

        let response = snapshot(&viewer);
        let device = response.device.unwrap();
        assert_eq!(device.missing_count, 5);
        assert_eq!(device.exchanges.len(), 1);
        assert_eq!(device.exchanges[0].peer_name, "bob");
        let who = device.slots.iter().find(|slot| slot.key == "who").unwrap();
        assert_eq!(who.text.as_deref(), Some("猫が"));
        assert_eq!(who.source_name.as_deref(), Some("bob"));
    }
}
