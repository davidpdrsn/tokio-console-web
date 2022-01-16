use crate::routes::ConsoleAddr;
use axum::{
    async_trait,
    extract::{FromRequest, Path, RequestParts},
    response::{IntoResponse, Response},
};
use axum_flash::IncomingFlashes;
use axum_live_view::{html, Html};
use std::convert::Infallible;

pub struct Layout {
    flash: IncomingFlashes,
}

impl Layout {
    pub fn render<T>(self, content: Html<T>) -> Html<T> {
        html! {
            <!DOCTYPE html>
            <html>
                <head>
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
                            , table.resources-table tr:nth-child(even) {
                                background: #eee;
                            }

                            table.tasks-table tr[axm-click]
                            , table.resources-table tr[axm-click]
                            {
                                cursor: pointer;
                            }

                            table.tasks-table tr[axm-click]:hover
                            , table.resources-table tr[axm-click]:hover
                            {
                                background: #ddd;
                            }
                        "#
                    </style>
                </head>
                <body>
                    if !self.flash.is_empty() {
                        for (_, msg) in self.flash {
                            <div>{ msg }</div>
                        }
                    }

                    <nav>
                        <a href="/">"Home"</a>
                    </nav>

                    <hr />

                    <div>
                        { content }
                    </div>

                    <script src="/assets/live-view.js"></script>
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
    type Rejection = Response;

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let flash = IncomingFlashes::from_request(req)
            .await
            .map_err(IntoResponse::into_response)?;

        Ok(Self { flash })
    }
}

pub struct TaskResourceLayout {
    layout: Layout,
    addr: ConsoleAddr,
}

impl TaskResourceLayout {
    pub fn render<T>(self, content: Html<T>) -> Html<T> {
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
