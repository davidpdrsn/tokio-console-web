use crate::{
    routes::ConsoleAddr,
    watch_stream::{ConsoleState, ConsoleStateWatch, Resource, ResourceId, TypeVisibility},
};
use axum::{
    async_trait,
    http::{HeaderMap, Uri},
};
use axum_live_view::{
    event_data::EventData,
    html, js_command,
    live_view::{Updated, ViewHandle},
    Html, LiveView,
};
use serde::{Deserialize, Serialize};

pub struct ResourcesIndex {
    rx: ConsoleStateWatch,
    paused_state: Option<ConsoleState>,
    addr: ConsoleAddr,
    connected: bool,
}

impl ResourcesIndex {
    pub fn new(addr: ConsoleAddr, rx: ConsoleStateWatch) -> Self {
        Self {
            addr,
            rx,
            paused_state: None,
            connected: true,
        }
    }
}

#[async_trait]
impl LiveView for ResourcesIndex {
    type Message = Msg;
    type Error = anyhow::Error;

    async fn mount(
        &mut self,
        _uri: Uri,
        _request_headers: &HeaderMap,
        handle: ViewHandle<Self::Message>,
    ) -> Result<(), Self::Error> {
        let mut rx = self.rx.clone();
        tokio::spawn(async move {
            loop {
                if rx.changed().await.is_err() {
                    break;
                }
                if handle.send(Msg::Update).await.is_err() {
                    break;
                }
            }
            let _ = handle.send(Msg::Disconnected).await;
            let _ = handle.send(Msg::Error).await;
        });
        Ok(())
    }

    async fn update(
        mut self,
        msg: Self::Message,
        _data: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        match msg {
            Msg::TogglePlayPause => {
                if self.paused_state.is_some() {
                    self.paused_state = None;
                } else {
                    self.paused_state = Some(self.rx.borrow().clone());
                }
                Ok(Updated::new(self))
            }
            Msg::RowClick(resource_id) => {
                let uri = format!(
                    "/console/{}/{}/resources/{}",
                    self.addr.ip, self.addr.port, resource_id.0
                )
                .parse()
                .expect("invalid URI");

                Ok(Updated::new(self).with(js_command::navigate_to(uri)))
            }
            Msg::Update => Ok(Updated::new(self)),
            Msg::Disconnected => {
                self.connected = false;
                Ok(Updated::new(self))
            }
            Msg::Error => {
                anyhow::bail!("console subscription disconnected")
            }
        }
    }

    fn render(&self) -> Html<Self::Message> {
        let state = self.rx.borrow();
        let state = if let Some(state) = &self.paused_state {
            state
        } else {
            &*state
        };

        html! {
            if self.connected {
                <div>
                    "Connection: " { &self.addr.ip } ":" { &self.addr.port }
                </div>
            } else {
                <div>
                    "Not connected..."
                </div>
            }

            <div>
                if self.paused_state.is_some() {
                    <button axm-click={ Msg::TogglePlayPause }>"Play"</button>
                } else {
                    <button axm-click={ Msg::TogglePlayPause }>"Pause"</button>
                }
            </div>

            <table class="resources-table">
                <thead>
                    <tr>
                        <th>"ID"</th>
                        <th>"Parent"</th>
                        <th>"Kind"</th>
                        <th>"Total"</th>
                        <th>"Target"</th>
                        <th>"Type"</th>
                        <th>"Vis"</th>
                        <th>"Location"</th>
                    </tr>
                </thead>
                <tbody>
                    for resource in state.resources.values() {
                        { resource.render_as_table_row() }
                    }
                </tbody>
            </table>
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    TogglePlayPause,
    RowClick(ResourceId),
    Update,
    Disconnected,
    Error,
}

impl Resource {
    fn render_as_table_row(&self) -> Html<Msg> {
        html! {
            <tr axm-click={ Msg::RowClick(self.id) }>
                <td>
                    { self.id.0 }
                </td>
                <td>
                    if let Some(parent_id) = self.parent_id {
                        { parent_id.0 }
                    }
                </td>
                <td>{ &self.kind }</td>
                <td>
                    if let Some(created_at) = self.stats.as_ref().and_then(|s| s.created_at) {
                        { format!("{:?}", created_at.elapsed().unwrap()) }
                    }
                </td>
                <td>
                    if let Some(target) = &self.target {
                        <code>{ target }</code>
                    }
                </td>
                <td>{ &self.concrete_type }</td>
                <td>
                    match self.vis {
                        TypeVisibility::Public => "✅",
                        TypeVisibility::Internal => "🔒",
                    }
                </td>
                <td>
                    <code>
                        if let Some(location) = &self.location {
                            { location.render() }
                        } else {
                            "{unknown location}"
                        }
                    </code>
                </td>
            </tr>
        }
    }
}
