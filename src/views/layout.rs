use crate::{routes::ConsoleAddr, Port};
use axum::{
    async_trait,
    extract::{Extension, FromRequest, Path, RequestParts},
};
use axum_liveview::{html, Html};
use std::convert::Infallible;

pub struct Layout {
    port: u16,
}

impl Layout {
    pub fn render(self, content: Html) -> Html {
        html! {
            <!DOCTYPE html>
            <html>
                <head>
                    { axum_liveview::assets() }
                    <style>
                        r#"
                            table {
                                border-collapse: collapse;
                                width: 100%;
                            }

                            th, td {
                                padding: 3px;
                            }

                            table.tasks-table tr:nth-child(even)
                            , table.resources-table tr:nth-child(even)
                            {
                                background: #eee;
                            }
                        "#
                    </style>
                </head>
                <body>
                    <nav>
                        <a href="/">"Home"</a>
                    </nav>

                    <hr />

                    <div>
                        { content }
                    </div>

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

pub struct TaskResourceLayout {
    layout: Layout,
    addr: ConsoleAddr,
}

impl TaskResourceLayout {
    pub fn render(self, content: Html) -> Html {
        self.layout.render(html! {
            <nav>
                <a href={ format!("/console/{}/{}/tasks", self.addr.ip, self.addr.port) }>"Tasks"</a>
                " | "
                <a href={ format!("/console/{}/{}/resources", self.addr.ip, self.addr.port) }>"Resources"</a>
            </nav>

            { content }
        })
    }
}

#[async_trait]
impl<B> FromRequest<B> for TaskResourceLayout
where
    B: Send,
{
    type Rejection = Infallible;

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let layout = Layout::from_request(req).await.unwrap();
        let Path(addr) = Path::<ConsoleAddr>::from_request(req).await.unwrap();

        Ok(Self { layout, addr })
    }
}
