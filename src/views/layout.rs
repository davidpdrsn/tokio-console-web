use crate::routes::ConsoleAddr;
use axum::extract::Path;
use axum_flash::IncomingFlashes;
use axum_live_view::{html, Html};

#[derive(axum_macros::FromRequest)]
#[from_request(rejection_derive(!Debug, !Display, !Error))]
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

                            table.resources-table tr:nth-child(even) {
                                background: #eee;
                            }

                            table.resources-table tr[axm-click] {
                                cursor: pointer;
                            }

                            table.resources-table tr[axm-click]:hover
                            , table.resources-table tr.row-selected
                            {
                                background: #ccc;
                            }

                            .keybinds {
                                margin: 0.5em 0;
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

#[derive(axum_macros::FromRequest)]
#[from_request(rejection_derive(!Debug, !Display, !Error))]
pub struct TaskResourceLayout {
    layout: Layout,
    #[from_request(via(Path))]
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
