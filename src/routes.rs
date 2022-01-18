use crate::views::ConnectionFailed;
use crate::watch_stream::ConsoleStateWatch;
use crate::{
    views::resources_index::ResourcesIndex, views::tasks_index::TasksIndex, views::Layout,
    views::TaskResourceLayout, watch_stream::ConsoleSubscriptions,
};
use axum::extract::Extension;
use axum::handler::Handler;
use axum::routing::MethodRouter;
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::get,
    Router,
};
use axum_flash::Flash;
use axum_live_view::{html, Html, LiveView, LiveViewUpgrade};
use serde::Deserialize;
use std::fmt;

pub fn all() -> Router {
    Router::new()
        .merge(root())
        .merge(open_console())
        .merge(tasks_index())
        .merge(resources_index())
        .fallback(fallback.into_service())
}

fn route(path: &str, method_router: MethodRouter) -> Router {
    Router::new().route(path, method_router)
}

async fn fallback(layout: Layout) -> (StatusCode, Html<()>) {
    let html = layout.render(html! {
        <p>"404 Not Found"</p>
    });
    (StatusCode::NOT_FOUND, html)
}

fn root() -> Router {
    async fn handler(layout: Layout, params: Option<Query<ConsoleAddr>>) -> impl IntoResponse {
        let Query(ConsoleAddr { ip, port }) = params.unwrap_or_default();

        layout.render::<()>(html! {
            <form method="GET" action="/open-console">
                <div>
                    <label>
                        <div>"IP"</div>
                        <input type="text" name="ip" required focus value={ ip }/>
                    </label>
                </div>

                <div>
                    <label>
                        <div>"Port"</div>
                        <input type="text" name="port" required value={ port }/>
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

impl Default for ConsoleAddr {
    fn default() -> Self {
        Self {
            ip: "127.0.0.1".to_owned(),
            port: "6669".to_owned(),
        }
    }
}

impl fmt::Display for ConsoleAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

fn open_console() -> Router {
    async fn handler(
        Query(addr): Query<ConsoleAddr>,
        Extension(subscriptions): Extension<ConsoleSubscriptions>,
        mut flash: Flash,
    ) -> impl IntoResponse {
        match subscriptions.subscribe(addr.clone()).await {
            Ok(_) => {
                let uri = format!("/console/{}/{}/tasks", addr.ip, addr.port)
                    .parse()
                    .unwrap();
                Redirect::to(uri)
            }
            Err(err) => {
                flash.error(format!("Failed to connect. Error: {}", err));
                let uri = format!("/?ip={ip}&port={port}", ip = addr.ip, port = addr.port)
                    .parse()
                    .unwrap();
                Redirect::to(uri)
            }
        }
    }

    route("/open-console", get(handler))
}

fn tasks_index() -> Router {
    route("/console/:ip/:port/tasks", get_state_view(TasksIndex::new))
}

fn resources_index() -> Router {
    route(
        "/console/:ip/:port/resources",
        get_state_view(ResourcesIndex::new),
    )
}

fn get_state_view<B, F, L>(make_view: F) -> MethodRouter<B>
where
    B: Send + 'static,
    F: Fn(ConsoleAddr, ConsoleStateWatch) -> L + Clone + Send + 'static,
    L: LiveView,
{
    get(
        |layout: TaskResourceLayout,
         live: LiveViewUpgrade,
         Extension(subscriptions): Extension<ConsoleSubscriptions>,
         Path(addr): Path<ConsoleAddr>| async move {
            match subscriptions.subscribe(addr.clone()).await {
                Ok(state) => Ok(live.response(|embed| {
                    let view = make_view(addr, state);
                    layout.render(embed.embed(view))
                })),
                Err(err) => Err(live
                    .response(|embed| layout.render(embed.embed(ConnectionFailed { addr, err })))),
            }
        },
    )
}
