use crate::views::ConnectionFailed;
use crate::{
    state::ConsoleSubscriptions, views::resources_index::ResourcesIndex,
    views::tasks_index::TasksIndex, views::Layout, views::TaskResourceLayout,
};
use axum::extract::Extension;
use axum::routing::MethodRouter;
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::get,
    Router,
};
use axum_live_view::{html, LiveViewUpgrade};
use serde::Deserialize;
use std::fmt;
use std::net::SocketAddr;

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

#[derive(Deserialize, PartialEq, Eq, Hash, Clone, Debug)]
pub struct ConsoleAddr {
    pub ip: String,
    pub port: String,
}

impl fmt::Display for ConsoleAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
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
        Extension(subscriptions): Extension<ConsoleSubscriptions>,
        Path(addr): Path<ConsoleAddr>,
    ) -> impl IntoResponse {
        match subscriptions.subscribe(addr.clone()).await {
            Ok(state) => Ok(live.response(|embed| {
                let view = TasksIndex::new(addr, state);
                layout.render(embed.embed(view))
            })),
            Err(err) => {
                Err(live
                    .response(|embed| layout.render(embed.embed(ConnectionFailed { addr, err }))))
            }
        }
    }

    route("/console/:ip/:port/tasks", get(handler))
}

fn resources_index() -> Router {
    async fn handler(
        layout: TaskResourceLayout,
        live: LiveViewUpgrade,
        Extension(subscriptions): Extension<ConsoleSubscriptions>,
        Path(addr): Path<ConsoleAddr>,
    ) -> impl IntoResponse {
        match subscriptions.subscribe(addr.clone()).await {
            Ok(state) => Ok(live.response(|embed| {
                let view = ResourcesIndex::new(addr, state);
                layout.render(embed.embed(view))
            })),
            Err(err) => {
                Err(live
                    .response(|embed| layout.render(embed.embed(ConnectionFailed { addr, err }))))
            }
        }
    }

    route("/console/:ip/:port/resources", get(handler))
}
