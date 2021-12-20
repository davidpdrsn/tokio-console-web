use axum::{AddExtensionLayer, Router};
use axum_liveview::{pubsub::InProcess, PubSub};
use clap::Parser;
use std::{net::SocketAddr, time::Duration};
use tower::ServiceBuilder;
use tower_http::ServiceBuilderExt;
use tracing_subscriber::{prelude::*, EnvFilter};

mod cancel_on_drop;
mod routes;
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

    let pubsub = InProcess::new();

    tokio::spawn(send_ticks(pubsub.clone()));

    let app = Router::new()
        .merge(routes::all())
        .merge(axum_liveview::routes())
        .layer(
            ServiceBuilder::new()
                .layer(AddExtensionLayer::new(Port(config.bind_addr.port())))
                .layer(AddExtensionLayer::new(pubsub.clone()))
                .layer(axum_liveview::layer(pubsub))
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

async fn send_ticks(pubsub: InProcess) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let _ = pubsub.broadcast("tick", ()).await;
    }
}
