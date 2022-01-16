use crate::state::ConsoleSubscriptions;
use axum::Router;
use axum_flash::Key;
use clap::Parser;
use std::net::SocketAddr;
use tower::ServiceBuilder;
use tower_http::ServiceBuilderExt;
use tracing_subscriber::{prelude::*, EnvFilter};

mod routes;
mod state;
mod views;

#[derive(Debug, Parser)]
struct Config {
    #[clap(long, env = "TOKIO_CONSOLE_BIND_ADDR", default_value = "0.0.0.0:3000")]
    bind_addr: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::default()
                .add_directive("tower_http=trace".parse()?)
                .add_directive("axum_live_view=trace".parse()?)
                .add_directive("tokio_console_web=trace".parse()?),
        )
        .init();

    let config = Config::parse();
    tracing::trace!(?config);

    let key = Key::generate();

    let app = Router::new()
        .merge(routes::all())
        .route("/assets/live-view.js", axum_live_view::precompiled_js())
        .layer(
            ServiceBuilder::new()
                .add_extension(ConsoleSubscriptions::default())
                .layer(
                    axum_flash::layer(key)
                        .use_secure_cookies(false)
                        .with_cookie_manager(),
                )
                .trace_for_http(),
        );

    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

type InstrumentClient =
    console_api::instrument::instrument_client::InstrumentClient<tonic::transport::Channel>;
