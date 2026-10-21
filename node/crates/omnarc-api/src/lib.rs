//! The control API of an Omnarc node (`protocol/CLUSTER.md` §5): `ClusterService` over the Connect protocol, and the
//! panel's files, on one HTTP port that by default only the device itself can reach.
//!
//! The node implements [`Cluster`]; [`router`] turns it into an axum router. This crate speaks the part of the
//! Connect protocol the panel and the apps use: unary calls and server streams, with binary Protobuf messages
//! (`application/proto`, `application/connect+proto`) and Connect's JSON error bodies.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use futures_util::future::BoxFuture;
use futures_util::stream::{BoxStream, StreamExt as _};
use omnarc_protocol::v1;
use prost::Message;

/// The path prefix of the service's procedures.
pub const SERVICE: &str = "/omnarc.v1.ClusterService";

/// The largest request body the API reads.
const MAX_REQUEST: usize = 1 << 20;

/// A Connect error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    InvalidArgument,
    NotFound,
    FailedPrecondition,
    PermissionDenied,
    Unimplemented,
    Internal,
    /// What was asked for is not there right now, such as a model the cluster no longer runs.
    Unavailable,
}

impl Code {
    const fn name(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::NotFound => "not_found",
            Self::FailedPrecondition => "failed_precondition",
            Self::PermissionDenied => "permission_denied",
            Self::Unimplemented => "unimplemented",
            Self::Internal => "internal",
            Self::Unavailable => "unavailable",
        }
    }

    const fn status(self) -> StatusCode {
        match self {
            Self::InvalidArgument | Self::FailedPrecondition => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::PermissionDenied => StatusCode::FORBIDDEN,
            Self::Unimplemented => StatusCode::NOT_IMPLEMENTED,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

/// An error of a call, sent as Connect's JSON error body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: Code,
    pub message: String,
}

impl Error {
    /// An error with `code` and a message for people.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "code": self.code.name(), "message": self.message })
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut response = (self.code.status(), self.json().to_string()).into_response();
        response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        response
    }
}

/// What the API asks of the node.
pub trait Cluster: Send + Sync + 'static {
    /// The cluster as this node knows it now.
    fn snapshot(&self) -> v1::ClusterSnapshot;

    /// What happens from now on; the API sends a snapshot first.
    fn watch(&self) -> BoxStream<'static, v1::WatchClusterResponse>;

    /// A one-time ticket that adds a device, valid for `ttl_seconds` (the node caps it at 10 minutes).
    ///
    /// # Errors
    /// The node cannot admit devices.
    fn create_ticket(&self, ttl_seconds: u32) -> Result<v1::CreateTicketResponse, Error>;

    /// The owner's answer for a device that asked to join; whether the device was still waiting.
    ///
    /// # Errors
    /// The request names no device.
    fn answer_join(&self, request: &v1::AnswerJoinRequest) -> Result<bool, Error>;

    /// Removes a member from the cluster, once the cluster agrees.
    ///
    /// # Errors
    /// This node is no admin, the member is unknown, or the cluster cannot agree now.
    fn remove_member(&self, member: v1::NodeId) -> BoxFuture<'static, Result<(), Error>>;

    /// Renames the cluster or changes its relays, once the cluster agrees.
    ///
    /// # Errors
    /// This node is no admin, a relay is not one, or the cluster cannot agree now.
    fn update_cluster(&self, request: v1::UpdateClusterRequest) -> BoxFuture<'static, Result<(), Error>> {
        let _ = request;
        Box::pin(async { Err(Error::new(Code::FailedPrecondition, "this device belongs to no cluster yet")) })
    }

    /// On a device that belongs to no cluster: creates one. A node in a cluster refuses.
    ///
    /// # Errors
    /// The device belongs to a cluster already, or the cluster cannot be written.
    fn create_cluster(&self, request: v1::CreateClusterRequest) -> BoxFuture<'static, Result<(), Error>> {
        let _ = request;
        Box::pin(async { Err(Error::new(Code::FailedPrecondition, "this device belongs to a cluster already")) })
    }

    /// On a device that belongs to no cluster: joins one with a ticket — the four symbols, then the cluster's name.
    /// A node in a cluster refuses.
    fn join_cluster(
        &self,
        request: v1::JoinClusterRequest,
    ) -> BoxStream<'static, Result<v1::JoinClusterResponse, Error>> {
        let _ = request;
        futures_util::stream::once(async {
            Err(Error::new(Code::FailedPrecondition, "this device belongs to a cluster already"))
        })
        .boxed()
    }

    /// What this device offers the cluster from now on (REQUIREMENTS Q1).
    ///
    /// # Errors
    /// The budget is not one, or it cannot be kept.
    fn set_budget(&self, budget: v1::Budget) -> Result<(), Error> {
        let _ = budget;
        Err(Error::new(Code::FailedPrecondition, "this device belongs to no cluster yet"))
    }

    /// Places a model within what every member offers, and starts it.
    ///
    /// # Errors
    /// The model is not here, the members do not offer enough, or the cluster cannot agree now.
    fn run_model(&self, request: v1::RunModelRequest) -> BoxFuture<'static, Result<v1::RunModelResponse, Error>> {
        let _ = request;
        Box::pin(async { Err(Error::new(Code::FailedPrecondition, "this device belongs to no cluster yet")) })
    }

    /// Stops the model that runs.
    ///
    /// # Errors
    /// The cluster cannot agree now.
    fn stop_model(&self) -> BoxFuture<'static, Result<(), Error>> {
        Box::pin(async { Err(Error::new(Code::FailedPrecondition, "this device belongs to no cluster yet")) })
    }

    /// Metrics in the Prometheus text format, for a scraper on this device (AP-08); nothing is sent anywhere.
    fn metrics(&self) -> String {
        String::new()
    }

    /// The models this device holds.
    fn list_models(&self) -> BoxFuture<'static, Result<v1::ListModelsResponse, Error>> {
        Box::pin(async { Ok(v1::ListModelsResponse::default()) })
    }

    /// Asks the running model; the answer streams piece by piece.
    fn ask(&self, request: v1::AskRequest) -> BoxStream<'static, Result<v1::AskResponse, Error>> {
        let _ = request;
        futures_util::stream::once(async { Err(Error::new(Code::FailedPrecondition, "no model runs here")) }).boxed()
    }
}

/// The API's routes for `cluster`, and the panel's files from `panel` when given.
pub fn router(cluster: Arc<dyn Cluster>, panel: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route(&format!("{SERVICE}/GetCluster"), post(get_cluster))
        .route(&format!("{SERVICE}/WatchCluster"), post(watch_cluster))
        .route(&format!("{SERVICE}/CreateTicket"), post(create_ticket))
        .route(&format!("{SERVICE}/AnswerJoin"), post(answer_join))
        .route(&format!("{SERVICE}/RemoveMember"), post(remove_member))
        .route(&format!("{SERVICE}/UpdateCluster"), post(update_cluster))
        .route(&format!("{SERVICE}/CreateCluster"), post(create_cluster))
        .route(&format!("{SERVICE}/JoinCluster"), post(join_cluster))
        .route(&format!("{SERVICE}/SetBudget"), post(set_budget))
        .route(&format!("{SERVICE}/RunModel"), post(run_model))
        .route(&format!("{SERVICE}/StopModel"), post(stop_model))
        .route(&format!("{SERVICE}/Ask"), post(ask))
        .route(&format!("{SERVICE}/ListModels"), post(list_models))
        .route("/metrics", axum::routing::get(metrics))
        .with_state(cluster);
    match panel {
        Some(dir) => {
            let index = dir.join("index.html");
            api.fallback_service(
                tower_http::services::ServeDir::new(dir).fallback(tower_http::services::ServeFile::new(index)),
            )
        }
        None => api,
    }
}

type ClusterState = State<Arc<dyn Cluster>>;

async fn get_cluster(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    unary(&headers, &body, |_: v1::GetClusterRequest| Ok(v1::GetClusterResponse { cluster: Some(cluster.snapshot()) }))
}

async fn create_ticket(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    unary(&headers, &body, |request: v1::CreateTicketRequest| cluster.create_ticket(request.ttl_seconds))
}

async fn answer_join(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    unary(&headers, &body, |request: v1::AnswerJoinRequest| {
        cluster.answer_join(&request).map(|waited| v1::AnswerJoinResponse { waited })
    })
}

async fn set_budget(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    unary(&headers, &body, |request: v1::SetBudgetRequest| {
        cluster.set_budget(request.budget.unwrap_or_default()).map(|()| v1::SetBudgetResponse {})
    })
}

async fn run_model(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    let request: v1::RunModelRequest = match decode(&headers, &body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    match cluster.run_model(request).await {
        Ok(response) => respond(&response),
        Err(error) => error.into_response(),
    }
}

async fn metrics(State(cluster): ClusterState) -> Response {
    let mut response = cluster.metrics().into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4"));
    response
}

async fn list_models(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(error) = decode::<v1::ListModelsRequest>(&headers, &body) {
        return error.into_response();
    }
    match cluster.list_models().await {
        Ok(response) => respond(&response),
        Err(error) => error.into_response(),
    }
}

async fn stop_model(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(error) = decode::<v1::StopModelRequest>(&headers, &body) {
        return error.into_response();
    }
    match cluster.stop_model().await {
        Ok(()) => respond(&v1::StopModelResponse {}),
        Err(error) => error.into_response(),
    }
}

async fn ask(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    if !content_type_is(&headers, "application/connect+proto") {
        return Error::new(
            Code::Unimplemented,
            "a stream speaks Connect with binary messages (application/connect+proto)",
        )
        .into_response();
    }
    let request: v1::AskRequest = match open_envelope(&body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    stream_response(cluster.ask(request))
}

async fn remove_member(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    let request: v1::RemoveMemberRequest = match decode(&headers, &body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    let Some(member) = request.member else {
        return Error::new(Code::InvalidArgument, "the request names no member").into_response();
    };
    match cluster.remove_member(member).await {
        Ok(()) => respond(&v1::RemoveMemberResponse {}),
        Err(error) => error.into_response(),
    }
}

async fn update_cluster(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    let request: v1::UpdateClusterRequest = match decode(&headers, &body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    match cluster.update_cluster(request).await {
        Ok(()) => respond(&v1::UpdateClusterResponse {}),
        Err(error) => error.into_response(),
    }
}

async fn create_cluster(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    let request: v1::CreateClusterRequest = match decode(&headers, &body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    match cluster.create_cluster(request).await {
        Ok(()) => respond(&v1::CreateClusterResponse {}),
        Err(error) => error.into_response(),
    }
}

async fn join_cluster(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    if !content_type_is(&headers, "application/connect+proto") {
        return Error::new(
            Code::Unimplemented,
            "a stream speaks Connect with binary messages (application/connect+proto)",
        )
        .into_response();
    }
    let request: v1::JoinClusterRequest = match open_envelope(&body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    stream_response(cluster.join_cluster(request))
}

// The messages of a server stream, then Connect's end of the stream: empty, or the error that ended it.
fn stream_response<M: Message + Send + 'static>(steps: BoxStream<'static, Result<M, Error>>) -> Response {
    let frames = futures_util::stream::unfold((steps, false), |(mut steps, done)| async move {
        if done {
            return None;
        }
        Some(match steps.next().await {
            Some(Ok(step)) => (envelope(0, &step.encode_to_vec()), (steps, false)),
            Some(Err(error)) => {
                (envelope(2, serde_json::json!({ "error": error.json() }).to_string().as_bytes()), (steps, true))
            }
            None => (envelope(2, b"{}"), (steps, true)),
        })
    })
    .map(Ok::<Bytes, std::convert::Infallible>);
    let mut response = Response::new(Body::from_stream(frames));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/connect+proto"));
    response
}

// The request of a unary call: one binary Protobuf message.
fn decode<Req: Message + Default>(headers: &HeaderMap, body: &[u8]) -> Result<Req, Error> {
    if !content_type_is(headers, "application/proto") {
        return Err(Error::new(
            Code::Unimplemented,
            "this API speaks Connect with binary messages (application/proto)",
        ));
    }
    if body.len() > MAX_REQUEST {
        return Err(Error::new(Code::InvalidArgument, "the request is too large"));
    }
    Req::decode(body).map_err(|e| Error::new(Code::InvalidArgument, format!("the request is malformed: {e}")))
}

fn respond<Resp: Message>(response: &Resp) -> Response {
    let mut response = (StatusCode::OK, response.encode_to_vec()).into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/proto"));
    response
}

// A unary call: one binary Protobuf message in, one out.
fn unary<Req: Message + Default, Resp: Message>(
    headers: &HeaderMap,
    body: &[u8],
    call: impl FnOnce(Req) -> Result<Resp, Error>,
) -> Response {
    match decode(headers, body).and_then(call) {
        Ok(response) => respond(&response),
        Err(error) => error.into_response(),
    }
}

async fn watch_cluster(State(cluster): ClusterState, headers: HeaderMap, body: Bytes) -> Response {
    if !content_type_is(&headers, "application/connect+proto") {
        return Error::new(
            Code::Unimplemented,
            "a stream speaks Connect with binary messages (application/connect+proto)",
        )
        .into_response();
    }
    if let Err(error) = open_envelope::<v1::WatchClusterRequest>(&body) {
        return error.into_response();
    }
    let first =
        v1::WatchClusterResponse { event: Some(v1::watch_cluster_response::Event::Snapshot(cluster.snapshot())) };
    let events = futures_util::stream::once(async move { first })
        .chain(cluster.watch())
        .map(|event| Ok::<Bytes, std::convert::Infallible>(envelope(0, &event.encode_to_vec())));
    let mut response = Response::new(Body::from_stream(events));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/connect+proto"));
    response
}

fn content_type_is(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next().is_some_and(|kind| kind.trim().eq_ignore_ascii_case(expected)))
}

/// A Connect envelope: flags, the length as 32-bit big-endian, and the bytes.
#[must_use]
pub fn envelope(flags: u8, message: &[u8]) -> Bytes {
    let mut bytes = Vec::with_capacity(5 + message.len());
    bytes.push(flags);
    bytes.extend_from_slice(&u32::try_from(message.len()).unwrap_or(u32::MAX).to_be_bytes());
    bytes.extend_from_slice(message);
    Bytes::from(bytes)
}

// The single message of an enveloped request body.
fn open_envelope<M: Message + Default>(body: &[u8]) -> Result<M, Error> {
    if body.len() < 5 || body.len() > MAX_REQUEST {
        return Err(Error::new(Code::InvalidArgument, "the request is not one Connect envelope"));
    }
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    if body[0] != 0 || body.len() != 5 + len {
        return Err(Error::new(Code::InvalidArgument, "the request is not one uncompressed Connect envelope"));
    }
    M::decode(&body[5..]).map_err(|e| Error::new(Code::InvalidArgument, format!("the request is malformed: {e}")))
}

/// Calls the API of the node on this device, as the command line does while the node's service runs.
pub mod client {
    use std::net::SocketAddr;

    use prost::Message;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::SERVICE;

    /// The largest answer read.
    const MAX_RESPONSE: u64 = 16 << 20;

    /// Calls `procedure` (such as `GetCluster`) of the API at `api` with `request`.
    ///
    /// # Errors
    /// No service answers there, or the call fails; the error is the service's message.
    pub async fn call<Req: Message, Resp: Message + Default>(
        api: SocketAddr,
        procedure: &str,
        request: &Req,
    ) -> Result<Resp, String> {
        let body = request.encode_to_vec();
        let mut stream = tokio::net::TcpStream::connect(api)
            .await
            .map_err(|e| format!("no Omnarc service answers on {api} ({e}): is omnarc-node serve running?"))?;
        let head = format!(
            "POST {SERVICE}/{procedure} HTTP/1.1\r\nHost: {api}\r\nContent-Type: application/proto\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let io = |e: std::io::Error| format!("the call to {api} broke: {e}");
        stream.write_all(head.as_bytes()).await.map_err(io)?;
        stream.write_all(&body).await.map_err(io)?;
        let mut response = Vec::new();
        stream.take(MAX_RESPONSE).read_to_end(&mut response).await.map_err(io)?;
        let (status, chunked, body) = parse(&response).ok_or("the service answered something that is not HTTP")?;
        let body = if chunked { dechunk(body).ok_or("a malformed chunked answer")? } else { body.to_vec() };
        if status == 200 {
            return Resp::decode(body.as_slice()).map_err(|e| format!("a malformed answer: {e}"));
        }
        let message = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|error| error.get("message").and_then(|m| m.as_str()).map(str::to_owned))
            .unwrap_or_else(|| format!("the service answered {status}"));
        Err(message)
    }

    // The status, whether the body is chunked, and the body.
    fn parse(response: &[u8]) -> Option<(u16, bool, &[u8])> {
        let end = response.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = std::str::from_utf8(&response[..end]).ok()?;
        let status = head.split_whitespace().nth(1)?.parse().ok()?;
        let chunked = head.lines().any(|line| {
            let line = line.to_ascii_lowercase();
            line.starts_with("transfer-encoding:") && line.contains("chunked")
        });
        Some((status, chunked, &response[end + 4..]))
    }

    fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let line_end = body.windows(2).position(|w| w == b"\r\n")?;
            let size =
                usize::from_str_radix(std::str::from_utf8(&body[..line_end]).ok()?.split(';').next()?.trim(), 16)
                    .ok()?;
            body = &body[line_end + 2..];
            if size == 0 {
                return Some(out);
            }
            out.extend_from_slice(body.get(..size)?);
            body = body.get(size + 2..)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    struct Fake;

    impl Cluster for Fake {
        fn snapshot(&self) -> v1::ClusterSnapshot {
            v1::ClusterSnapshot { name: "Home cluster".into(), can_invite: true, ..Default::default() }
        }

        fn watch(&self) -> BoxStream<'static, v1::WatchClusterResponse> {
            let telemetry = v1::WatchClusterResponse {
                event: Some(v1::watch_cluster_response::Event::Telemetry(v1::Telemetry {
                    cpu_load: 0.25,
                    ..Default::default()
                })),
            };
            futures_util::stream::iter([telemetry]).chain(futures_util::stream::pending()).boxed()
        }

        fn create_ticket(&self, ttl_seconds: u32) -> Result<v1::CreateTicketResponse, Error> {
            if ttl_seconds == 0 {
                return Err(Error::new(Code::InvalidArgument, "a ticket lives at least a second"));
            }
            Ok(v1::CreateTicketResponse { qr_payload: "OMNARC:ABC".into(), ..Default::default() })
        }

        fn answer_join(&self, request: &v1::AnswerJoinRequest) -> Result<bool, Error> {
            Ok(request.matches)
        }

        fn remove_member(&self, member: v1::NodeId) -> BoxFuture<'static, Result<(), Error>> {
            Box::pin(async move {
                if member.ed25519_public_key == [9; 32] {
                    Ok(())
                } else {
                    Err(Error::new(Code::NotFound, "no such member"))
                }
            })
        }
    }

    fn app() -> Router {
        router(Arc::new(Fake), None)
    }

    #[tokio::test]
    async fn the_command_line_calls_the_api() {
        use std::future::IntoFuture as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app()).into_future());

        let got: v1::GetClusterResponse = client::call(addr, "GetCluster", &v1::GetClusterRequest {}).await.unwrap();
        assert_eq!(got.cluster.unwrap(), Fake.snapshot());
        let member = |byte| v1::RemoveMemberRequest { member: Some(v1::NodeId { ed25519_public_key: vec![byte; 32] }) };
        let _: v1::RemoveMemberResponse = client::call(addr, "RemoveMember", &member(9)).await.unwrap();
        let refused = client::call::<_, v1::RemoveMemberResponse>(addr, "RemoveMember", &member(1)).await;
        assert_eq!(refused.unwrap_err(), "no such member");

        // nothing listens on port 1
        let nobody = client::call::<_, v1::GetClusterResponse>(
            "127.0.0.1:1".parse().unwrap(),
            "GetCluster",
            &v1::GetClusterRequest {},
        );
        assert!(nobody.await.unwrap_err().contains("is omnarc-node serve running"));
    }

    fn call(path: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
        Request::post(format!("{SERVICE}/{path}"))
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn a_unary_call_answers_in_binary() {
        let response = app()
            .oneshot(call("GetCluster", "application/proto", v1::GetClusterRequest {}.encode_to_vec()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/proto");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let cluster = v1::GetClusterResponse::decode(body).unwrap().cluster.unwrap();
        assert_eq!(cluster.name, "Home cluster");
        assert!(cluster.can_invite);
    }

    #[tokio::test]
    async fn an_error_is_connect_json() {
        let request = v1::CreateTicketRequest { ttl_seconds: 0 }.encode_to_vec();
        let response = app().oneshot(call("CreateTicket", "application/proto", request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["code"], "invalid_argument");

        let json = app().oneshot(call("GetCluster", "application/json", b"{}".to_vec())).await.unwrap();
        assert_eq!(json.status(), StatusCode::NOT_IMPLEMENTED, "only binary messages");
        let garbage = app().oneshot(call("GetCluster", "application/proto", vec![0xff; 3])).await.unwrap();
        assert_eq!(garbage.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_stream_starts_with_a_snapshot() {
        let request = envelope(0, &v1::WatchClusterRequest {}.encode_to_vec()).to_vec();
        let response = app().oneshot(call("WatchCluster", "application/connect+proto", request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        let mut buffer = Vec::new();
        let mut events = Vec::new();
        while events.len() < 2 {
            let frame = body.frame().await.unwrap().unwrap();
            buffer.extend_from_slice(&frame.into_data().unwrap());
            while buffer.len() >= 5 {
                let len = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;
                if buffer.len() < 5 + len {
                    break;
                }
                assert_eq!(buffer[0], 0, "a message, not the end of the stream");
                events.push(v1::WatchClusterResponse::decode(&buffer[5..5 + len]).unwrap());
                buffer.drain(..5 + len);
            }
        }
        assert!(matches!(
            &events[0].event,
            Some(v1::watch_cluster_response::Event::Snapshot(s)) if s.name == "Home cluster"
        ));
        assert!(matches!(
            &events[1].event,
            Some(v1::watch_cluster_response::Event::Telemetry(t)) if (t.cpu_load - 0.25).abs() < 1e-6
        ));
    }

    #[tokio::test]
    async fn a_malformed_stream_request_is_refused() {
        let response = app().oneshot(call("WatchCluster", "application/connect+proto", vec![0, 0, 0])).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
