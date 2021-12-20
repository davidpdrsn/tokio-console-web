use axum::{
    async_trait,
    extract::{Extension, FromRequest, RequestParts},
    response::IntoResponse,
    routing::get,
    AddExtensionLayer, Router,
};
use axum_liveview::{html, pubsub::InProcess, Html};
use clap::Parser;
use std::{convert::Infallible, net::SocketAddr};
use tower::ServiceBuilder;

#[derive(Debug, Parser)]
struct Opt {
    #[clap(long, env = "TOKIO_CONSOLE_BIND_ADDR", default_value = "0.0.0.0:3000")]
    bind_addr: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    tracing::trace!(?opt);

    let pubsub = InProcess::new();

    let app = Router::new()
        .merge(root())
        .merge(axum_liveview::routes())
        .layer(
            ServiceBuilder::new()
                .layer(AddExtensionLayer::new(Port(opt.bind_addr.port())))
                .layer(axum_liveview::layer(pubsub)),
        );

    axum::Server::bind(&opt.bind_addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn root() -> Router {
    async fn handler(layout: Layout) -> impl IntoResponse {
        layout.render(html! { "Hi from 'GET /'" })
    }

    Router::new().route("/", get(handler))
}

#[derive(Copy, Clone)]
struct Port(u16);

struct Layout {
    port: u16,
}

impl Layout {
    fn render(self, content: Html) -> Html {
        html! {
            <!DOCTYPE html>
            <html>
                <head>
                    { axum_liveview::assets() }
                </head>
                <body>
                    { content }

                    <script>
                        {
                            format!(
                                r#"
                                    const liveView = new LiveView('localhost', {})
                                    liveView.connect()
                                "#,
                                self.port,
                            )
                        }
                    </script>
                </body>
            </html>
        }
    }
}

#[async_trait]
impl<B> FromRequest<B> for Layout
where
    B: Send,
{
    type Rejection = Infallible;

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let Extension(Port(port)) = FromRequest::from_request(req).await.unwrap();

        Ok(Self { port })
    }
}
