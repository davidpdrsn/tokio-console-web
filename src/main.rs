use axum::Router;
use clap::Parser;
use std::net::SocketAddr;
use tower::ServiceBuilder;
use tower_http::ServiceBuilderExt;
use tracing_subscriber::{prelude::*, EnvFilter};

use crate::state::ConsoleSubscriptions;

mod routes;
mod state;
mod views;

#[derive(Debug, Parser)]
struct Config {
    #[clap(long, env = "TOKIO_CONSOLE_BIND_ADDR", default_value = "0.0.0.0:3000")]
    bind_addr: SocketAddr,

    #[clap(
        long,
        env = "TOKIO_CONSOLE_ADDR",
        default_value = "http://127.0.0.1:6669"
    )]
    console_addr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::default()
                .add_directive("tower_http=trace".parse()?)
                .add_directive("tokio_console_web=trace".parse()?),
        )
        .init();

    let config = Config::parse();
    tracing::trace!(?config);

    let app = Router::new()
        .merge(routes::all())
        .route("/assets/live-view.js", axum_live_view::precompiled_js())
        .layer(
            ServiceBuilder::new()
                .add_extension(Port(config.bind_addr.port()))
                .add_extension(ConsoleSubscriptions::default())
                .trace_for_http(),
        );

    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

#[derive(Copy, Clone)]
struct Port(u16);

type InstrumentClient =
    console_api::instrument::instrument_client::InstrumentClient<tonic::transport::Channel>;
