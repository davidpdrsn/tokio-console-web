use crate::cancel_on_drop::CancelOnDropChildToken;
use crate::views::{connection_state, resources_index, tasks_index, TaskResourceLayout};
use crate::InstrumentClient;
use crate::{cancel_on_drop::CancelOnDrop, views::Layout};
use axum::routing::MethodRouter;
use axum::{
    async_trait,
    extract::{FromRequest, Path, Query, RequestParts},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_live_view::{html, LiveViewUpgrade};
use serde::Deserialize;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tonic::transport::Endpoint;

pub fn all() -> Router {
    Router::new()
        .merge(root())
        .merge(open_console())
        .merge(tasks_index())
        .merge(resources_index())
}

fn route(path: &str, method_router: MethodRouter) -> Router {
    Router::new().route(path, method_router)
}

fn root() -> Router {
    async fn handler(layout: Layout) -> impl IntoResponse {
        layout.render::<()>(html! {
            <form method="GET" action="/open-console">
                <div>
                    <label>
                        <div>"IP"</div>
                        <input type="text" name="ip" required focus value="127.0.0.1" />
                    </label>
                </div>

                <div>
                    <label>
                        <div>"Port"</div>
                        <input type="text" name="port" required value="6669" />
                    </label>
                </div>

                <input type="submit" value="Go" />
            </form>
        })
    }

    route("/", get(handler))
}

#[derive(Deserialize, Clone, Debug)]
pub struct ConsoleAddr {
    pub ip: String,
    pub port: String,
}

fn open_console() -> Router {
    async fn handler(
        Query(params): Query<ConsoleAddr>,
    ) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
        let _ = format!("{}:{}", params.ip, params.port)
            .parse::<SocketAddr>()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid IP or port"))?;

        let uri = format!("/console/{}/{}/tasks", params.ip, params.port)
            .parse()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Failed to generate URI"))?;

        Ok(Redirect::to(uri))
    }

    route("/open-console", get(handler))
}

fn tasks_index() -> Router {
    async fn handler(
        layout: TaskResourceLayout,
        live: LiveViewUpgrade,
        ConnectedClient(client): ConnectedClient,
        Path(addr): Path<ConsoleAddr>,
    ) -> impl IntoResponse {
        live.response(|embed| {
            let token = CancelOnDrop::new();

            let (connection_state_tx, connection_state_rx) = mpsc::channel(1024);
            let (msg_tx, msg_rx) = mpsc::channel(1024);

            if embed.connected() {
                tokio::spawn(process_tasks_index_stream(
                    token.child(),
                    client,
                    connection_state_tx,
                    msg_tx,
                ));
            }

            let connection_state =
                connection_state::ConnectionState::new(addr.clone(), connection_state_rx);

            let view = tasks_index::TasksIndex::new(addr, token, msg_rx);

            let views = axum_live_view::live_view::combine(
                (connection_state, view),
                |connection_state, view| {
                    html! {
                        { connection_state }
                        { view }
                    }
                },
            );
            layout.render(embed.embed(views))
        })
    }

    route("/console/:ip/:port/tasks", get(handler))
}

fn resources_index() -> Router {
    async fn handler(
        layout: TaskResourceLayout,
        live: LiveViewUpgrade,
        ConnectedClient(client): ConnectedClient,
        Path(addr): Path<ConsoleAddr>,
    ) -> impl IntoResponse {
        live.response(|embed| {
            let token = CancelOnDrop::new();

            let (connection_state_tx, connection_state_rx) = mpsc::channel(1024);
            let (msg_tx, msg_rx) = mpsc::channel(1024);

            if embed.connected() {
                tokio::spawn(process_tasks_index_stream(
                    token.child(),
                    client,
                    connection_state_tx,
                    msg_tx,
                ));
            }

            let connection_state =
                connection_state::ConnectionState::new(addr.clone(), connection_state_rx);

            let view = resources_index::ResourcesIndex::new(addr, token, msg_rx);

            let views = axum_live_view::live_view::combine(
                (connection_state, view),
                |connection_state, view| {
                    html! {
                        { connection_state }
                        { view }
                    }
                },
            );
            layout.render(embed.embed(views))
        })
    }

    route("/console/:ip/:port/resources", get(handler))
}

#[allow(irrefutable_let_patterns)]
async fn process_tasks_index_stream<M>(
    token: CancelOnDropChildToken,
    mut client: InstrumentClient,
    connection_state_tx: mpsc::Sender<connection_state::Msg>,
    msg_tx: mpsc::Sender<M>,
) where
    M: TryFrom<console_api::instrument::Update, Error = anyhow::Error> + Send + Sync + 'static,
{
    use console_api::instrument::InstrumentRequest;

    let process_stream = async {
        let mut stream = match client.watch_updates(InstrumentRequest {}).await {
            Ok(res) => res.into_inner(),
            Err(err) => {
                let code: i32 = err.code().into();
                let message = err.message().to_owned();
                let _ = connection_state_tx
                    .send(connection_state::Msg::Error { code, message })
                    .await;
                return;
            }
        };

        let _ = connection_state_tx
            .send(connection_state::Msg::Connected)
            .await;

        while let msg = stream.message().await {
            match msg {
                Ok(Some(msg)) => match M::try_from(msg) {
                    Ok(msg) => {
                        let _ = msg_tx.send(msg).await;
                    }
                    Err(err) => {
                        tracing::error!(%err, "failed to convert gRPC message");
                        return;
                    }
                },
                Ok(None) => {
                    let _ = connection_state_tx
                        .send(connection_state::Msg::StreamEnded)
                        .await;
                    break;
                }
                Err(err) => {
                    let code: i32 = err.code().into();
                    let message = err.message().to_owned();
                    let _ = connection_state_tx
                        .send(connection_state::Msg::Error { code, message })
                        .await;
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = process_stream => {}
        _ = token.cancelled() => {
            tracing::trace!("ending watch_update stream");
        }
    }
}

struct ConnectedClient(InstrumentClient);

#[async_trait]
impl<B> FromRequest<B> for ConnectedClient
where
    B: Send,
{
    type Rejection = Response;

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let Path(params) = Path::<ConsoleAddr>::from_request(req)
            .await
            .map_err(|err| err.into_response())?;

        let endpoint = format!("http://{}:{}", params.ip, params.port)
            .parse::<Endpoint>()
            .map_err(|err| {
                tracing::error!(%err, "Invalid endpoint for gRPC client");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Invalid endpoint for gRPC client",
                )
                    .into_response()
            })?;

        let channel = endpoint.connect_lazy();

        let client = InstrumentClient::new(channel);

        Ok(Self(client))
    }
}
