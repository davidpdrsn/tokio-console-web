use crate::cancel_on_drop::CancelOnDropChildToken;
use crate::views::{connection_state, resources_index, tasks_index, TaskResourceLayout};
use crate::InstrumentClient;
use crate::{cancel_on_drop::CancelOnDrop, views::Layout};
use axum::{
    async_trait,
    extract::{Extension, FromRequest, Path, Query, RequestParts},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use axum_liveview::pubsub::Bincode;
use axum_liveview::{
    html,
    pubsub::{InProcess, PubSub},
    LiveViewManager,
};
use serde::Deserialize;
use std::net::SocketAddr;
use tonic::transport::Endpoint;
use uuid::Uuid;

pub fn all() -> Router {
    Router::new()
        .merge(root())
        .merge(open_console())
        .merge(tasks_index())
        .merge(resources_index())
}

fn root() -> Router {
    async fn handler(layout: Layout) -> impl IntoResponse {
        layout.render(html! {
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

    Router::new().route("/", get(handler))
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

    Router::new().route("/open-console", get(handler))
}

fn tasks_index() -> Router {
    async fn handler(
        layout: TaskResourceLayout,
        live: LiveViewManager,
        ConnectedClient(client): ConnectedClient,
        Extension(pubsub): Extension<InProcess>,
        Path(addr): Path<ConsoleAddr>,
    ) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
        let token = CancelOnDrop::new();

        let stream_id = Uuid::new_v4();

        tokio::spawn(process_tasks_index_stream::<tasks_index::Update>(
            token.child(),
            client,
            pubsub,
            stream_id,
            tasks_index::msg_topic(stream_id),
        ));

        let connection_state = connection_state::ConnectionState::new(stream_id, addr);
        let view = tasks_index::TasksIndex::new(token, stream_id);

        Ok(layout.render(html! {
            { live.embed(connection_state) }
            { live.embed(view) }
        }))
    }

    Router::new().route("/console/:ip/:port/tasks", get(handler))
}

fn resources_index() -> Router {
    async fn handler(
        layout: TaskResourceLayout,
        live: LiveViewManager,
        ConnectedClient(client): ConnectedClient,
        Extension(pubsub): Extension<InProcess>,
        Path(addr): Path<ConsoleAddr>,
    ) -> impl IntoResponse {
        let token = CancelOnDrop::new();
        let stream_id = Uuid::new_v4();

        tokio::spawn(process_tasks_index_stream::<resources_index::Update>(
            token.child(),
            client,
            pubsub,
            stream_id,
            resources_index::msg_topic(stream_id),
        ));

        let connection_state = connection_state::ConnectionState::new(stream_id, addr);
        let view = resources_index::ResourcesIndex::new(token, stream_id);

        layout.render(html! {
            { live.embed(connection_state) }
            { live.embed(view) }
        })
    }

    Router::new().route("/console/:ip/:port/resources", get(handler))
}

#[allow(irrefutable_let_patterns)]
async fn process_tasks_index_stream<T>(
    token: CancelOnDropChildToken,
    mut client: InstrumentClient,
    pubsub: InProcess,
    stream_id: Uuid,
    message_topic: String,
) where
    T: TryFrom<console_api::instrument::Update, Error = anyhow::Error>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    use console_api::instrument::InstrumentRequest;

    let process_stream = async move {
        let mut stream = match client.watch_updates(InstrumentRequest {}).await {
            Ok(res) => res.into_inner(),
            Err(err) => {
                let code: i32 = err.code().into();
                let message = err.message().to_owned();
                let _ = pubsub
                    .broadcast(
                        &connection_state::msg_topic(stream_id),
                        Bincode(connection_state::Msg::Error { code, message }),
                    )
                    .await;
                return;
            }
        };

        while let msg = stream.message().await {
            match msg {
                Ok(Some(msg)) => match T::try_from(msg) {
                    Ok(msg) => {
                        let _ = pubsub.broadcast(&message_topic, Bincode(msg)).await;
                    }
                    Err(err) => {
                        tracing::error!(%err, "failed to convert gRPC message");
                        return;
                    }
                },
                Ok(None) => {
                    let _ = pubsub
                        .broadcast(
                            &connection_state::msg_topic(stream_id),
                            Bincode(connection_state::Msg::StreamEnded),
                        )
                        .await;
                    break;
                }
                Err(err) => {
                    let code: i32 = err.code().into();
                    let message = err.message().to_owned();
                    let _ = pubsub
                        .broadcast(
                            &connection_state::msg_topic(stream_id),
                            Bincode(connection_state::Msg::Error { code, message }),
                        )
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
