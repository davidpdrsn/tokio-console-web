use std::{collections::HashMap, sync::Arc, time::Duration};

use super::{
    table::TableView,
    table_view_keybinds::{TableViewKeybinds, TableViewKeybindsUpdate},
    StateRef,
};
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
    html,
    js_command::{self, JsCommand},
    live_view::{Updated, ViewHandle},
    Html, LiveView,
};
use serde::{Deserialize, Serialize};

pub struct ResourcesIndex {
    rx: ConsoleStateWatch,
    paused_state: Option<ConsoleState>,
    addr: ConsoleAddr,
    connected: bool,
    table_keybinds: TableViewKeybinds,
    runtime_stats: HashMap<ResourceId, ResourceRuntimeStats>,
}

impl ResourcesIndex {
    pub fn new(addr: ConsoleAddr, rx: ConsoleStateWatch) -> Self {
        Self {
            addr,
            rx,
            paused_state: None,
            connected: true,
            table_keybinds: Default::default(),
            runtime_stats: Default::default(),
        }
    }
}

impl ResourcesIndex {
    fn state(&self) -> StateRef<'_, ConsoleState> {
        let state = self.rx.borrow();
        if let Some(state) = &self.paused_state {
            StateRef::Ref(state)
        } else {
            StateRef::BorrowedFromWatch(state)
        }
    }

    fn navigate_to_resource_command(&self, id: ResourceId) -> JsCommand {
        let uri = format!(
            "/console/{}/{}/resources/{}",
            self.addr.ip, self.addr.port, id.0
        )
        .parse()
        .expect("invalid URI");
        js_command::navigate_to(uri)
    }

    fn selected_resource(&self, idx: usize) -> Option<ResourceId> {
        let state = self.state();
        let resource = state.resources.values().nth(idx)?;
        Some(resource.id)
    }

    fn toggle_play_pause(&mut self) {
        if self.paused_state.is_some() {
            self.paused_state = None;
        } else {
            self.paused_state = Some(self.rx.borrow().clone());
        }
    }

    async fn do_update(
        mut self,
        msg: Msg,
        data: Option<EventData>,
    ) -> Result<Updated<Self>, anyhow::Error> {
        let mut commands = Vec::new();

        match msg {
            Msg::TogglePlayPause => {
                self.toggle_play_pause();
            }
            Msg::Key => match self.table_keybinds.update(data.as_ref()) {
                Some(TableViewKeybindsUpdate::Selected(idx)) => {
                    if let Some(id) = self.selected_resource(idx) {
                        commands.push(self.navigate_to_resource_command(id));
                    }
                }
                Some(TableViewKeybindsUpdate::TogglePlayPause) => {
                    self.toggle_play_pause();
                }
                Some(TableViewKeybindsUpdate::GotoResources) => {}
                Some(TableViewKeybindsUpdate::GotoTasks) => {
                    commands.push(js_command::navigate_to(
                        format!("/console/{}/{}/tasks", self.addr.ip, self.addr.port)
                            .parse()
                            .unwrap(),
                    ));
                }
                None => {}
            },
            Msg::RowClick(id) => {
                commands.push(self.navigate_to_resource_command(id));
            }
            Msg::Update => {
                if self.paused_state.is_none() {
                    for resource in self.rx.borrow().resources.values() {
                        let mut times = ResourceRuntimeStats::default();

                        if let Some(total) = resource
                            .stats
                            .as_ref()
                            .and_then(|s| s.created_at)
                            .map(|t| t.elapsed().unwrap())
                        {
                            times.total = Some(total);
                        }

                        self.runtime_stats.insert(resource.id, times);
                    }
                }
            }
            Msg::Disconnected => {
                self.connected = false;
            }
            Msg::Error => {
                anyhow::bail!("console subscription disconnected")
            }
        }

        let num_resources = self.state().resources.len();
        self.table_keybinds.clamp_selected_idx(num_resources);

        Ok(Updated::new(self).with_all(commands))
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
        data: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        self.do_update(msg, data).await
    }

    fn render(&self) -> Html<Self::Message> {
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

            { self.table_keybinds.help() }

            <div>
                if self.paused_state.is_some() {
                    <button axm-click={ Msg::TogglePlayPause }>"Play"</button>
                } else {
                    <button axm-click={ Msg::TogglePlayPause }>"Pause"</button>
                }
            </div>

            { self.table_render() }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    TogglePlayPause,
    RowClick(ResourceId),
    Key,
    Update,
    Disconnected,
    Error,
}

pub(crate) struct ResourceViewModel {
    resource: Arc<Resource>,
    runtime_stats: Option<ResourceRuntimeStats>,
}

impl TableView for ResourcesIndex {
    type Column = Column;
    type Model = ResourceViewModel;
    type Msg = Msg;

    fn columns(&self) -> Vec<Self::Column> {
        Column::all()
    }

    fn rows(&self) -> Vec<Self::Model> {
        self.state()
            .resources
            .values()
            .map(|resource| ResourceViewModel {
                resource: Arc::clone(resource),
                runtime_stats: self.runtime_stats.get(&resource.id).copied(),
            })
            .collect()
    }

    fn render_column(&self, col: &Self::Column, row: &Self::Model) -> Html<Self::Msg> {
        match col {
            Column::ID => {
                html! { { row.resource.id.0 } }
            }
            Column::Parent => {
                html! {
                    if let Some(parent_id) = row.resource.parent_id {
                        { parent_id.0 }
                    }
                }
            }
            Column::Kind => {
                html! { { &row.resource.kind } }
            }
            Column::Total => {
                html! {
                    if let Some(total) = row.runtime_stats.and_then(|t| t.total) {
                        { format!("{:?}", total) }
                    }
                }
            }
            Column::Target => {
                html! {
                    if let Some(target) = &row.resource.target {
                        <code>{ target }</code>
                    }
                }
            }
            Column::Type => {
                html! { { &row.resource.concrete_type } }
            }
            Column::Vis => {
                html! {
                    match row.resource.vis {
                        TypeVisibility::Public => "✅",
                        TypeVisibility::Internal => "🔒",
                    }
                }
            }
            Column::Location => {
                html! {
                    if let Some(location) = &row.resource.location {
                        { location.render() }
                    } else {
                        "{unknown location}"
                    }
                }
            }
        }
    }

    fn row_click_event(&self, row: &Self::Model) -> Self::Msg {
        Msg::RowClick(row.resource.id)
    }

    fn key_event(&self) -> Self::Msg {
        Msg::Key
    }

    fn row_selected(&self, idx: usize, _row: &Self::Model) -> bool {
        self.table_keybinds.selected_idx() == Some(idx)
    }
}

columns_enum! {
    pub(crate) enum Column {
        ID,
        Parent,
        Kind,
        Total,
        Target,
        Type,
        Vis,
        Location,
    }
}

#[derive(Default, Clone, Copy)]
struct ResourceRuntimeStats {
    total: Option<Duration>,
}
