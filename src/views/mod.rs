use crate::Port;
use axum::{
    async_trait,
    extract::{Extension, FromRequest, RequestParts},
};
use axum_liveview::{html, Html};
use std::convert::Infallible;

pub mod tasks_index;

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

                            table.tasks-table tr:nth-child(even) {
                                background: #ddd;
                            }
                        "#
                    </style>
                </head>
                <body>
                    <nav>
                        <a href="/">"Home"</a>
                    </nav>

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
