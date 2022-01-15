use super::{Location, MetaId, Metadata};
use crate::{cancel_on_drop::CancelOnDrop, routes::ConsoleAddr};
use anyhow::Context;
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
use std::{
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    time::{Duration, SystemTime},
};
use tokio::sync::mpsc;

pub struct ResourcesIndex {
    state: State,
    paused_state: Option<State>,
    addr: ConsoleAddr,
    rx: Option<mpsc::Receiver<Update>>,
    _token: CancelOnDrop,
}

#[derive(Clone)]
struct State {
    resources: BTreeMap<ResourceId, Resource>,
    metadata: HashMap<MetaId, Metadata>,
}

#[async_trait]
impl LiveView for ResourcesIndex {
    type Message = Msg;
    type Error = Infallible;

    async fn mount(
        &mut self,
        _uri: Uri,
        _request_headers: &HeaderMap,
        handle: ViewHandle<Self::Message>,
    ) -> Result<(), Self::Error> {
        if let Some(mut rx) = self.rx.take() {
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if handle.send(Msg::Update(msg)).await.is_err() {
                        break;
                    }
                }
            });
        }
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
                    self.paused_state = Some(self.state.clone());
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
            Msg::Update(update) => {
                let Update {
                    new_resources,
                    new_metadata,
                    stats_update,
                } = update;

                for resources in new_resources {
                    self.state.resources.insert(resources.id, resources);
                }

                self.state.metadata.extend(new_metadata);

                for (id, stats) in stats_update {
                    if let Some(resource) = self.state.resources.get_mut(&id) {
                        resource.stats = Some(stats);
                    }
                }

                for resource in self.state.resources.values_mut() {
                    if let Some(metadata) = self.state.metadata.get(&resource.metadata_id) {
                        resource.target = Some(metadata.target.clone());
                    }
                }

                self.state.resources.retain(|_id, resource| {
                    if let Some(stats) = &resource.stats {
                        if let Some(dropped_at) = stats.dropped_at {
                            dropped_at.elapsed().unwrap() < Duration::from_secs(5)
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                });

                Ok(Updated::new(self))
            }
        }
    }

    fn render(&self) -> Html<Self::Message> {
        let state = if let Some(state) = &self.paused_state {
            state
        } else {
            &self.state
        };

        html! {
            <div>
                if state.resources.is_empty() {
                    "Loading..."
                } else {
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
            </div>
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Msg {
    RowClick(ResourceId),
    TogglePlayPause,
    Update(Update),
}

impl ResourcesIndex {
    pub fn new(addr: ConsoleAddr, token: CancelOnDrop, rx: mpsc::Receiver<Update>) -> Self {
        Self {
            paused_state: None,
            state: State {
                resources: Default::default(),
                metadata: Default::default(),
            },
            addr,
            rx: Some(rx),
            _token: token,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Update {
    new_resources: Vec<Resource>,
    new_metadata: HashMap<MetaId, Metadata>,
    stats_update: BTreeMap<ResourceId, Stats>,
}

impl TryFrom<console_api::instrument::Update> for Update {
    type Error = anyhow::Error;

    fn try_from(update: console_api::instrument::Update) -> Result<Self, Self::Error> {
        let console_api::instrument::Update {
            resource_update,
            new_metadata,
            ..
        } = update;

        let resource_update = resource_update.context("Missing `resource_update` field")?;
        let console_api::resources::ResourceUpdate {
            new_resources,
            stats_update,
            new_poll_ops: _,
            dropped_events: _,
        } = resource_update;

        let new_resources = new_resources
            .into_iter()
            .map(Resource::try_from)
            .collect::<anyhow::Result<_>>()?;

        let new_metadata = new_metadata
            .unwrap_or_default()
            .metadata
            .into_iter()
            .map(|meta| Metadata::try_from(meta).map(|meta| (meta.id, meta)))
            .collect::<anyhow::Result<_>>()?;

        let stats_update = stats_update
            .into_iter()
            .map(|(id, stats)| Ok((ResourceId(id), stats.try_into()?)))
            .collect::<anyhow::Result<_>>()?;

        Ok(Self {
            new_resources,
            new_metadata,
            stats_update,
        })
    }
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
struct Resource {
    id: ResourceId,
    vis: TypeVisibility,
    parent_id: Option<ResourceId>,
    kind: String,
    concrete_type: String,
    location: Option<Location>,
    metadata_id: MetaId,
    target: Option<String>,
    stats: Option<Stats>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(u64);

impl TryFrom<console_api::resources::Resource> for Resource {
    type Error = anyhow::Error;

    fn try_from(resource: console_api::resources::Resource) -> Result<Self, Self::Error> {
        let id = ResourceId(resource.id.context("Missing `id` field")?.id);

        let metadata_id = MetaId(resource.metadata.context("Missing `metadata` field")?.id);

        let kind = match resource
            .kind
            .context("Missing `kind` field")?
            .kind
            .context("Missing `kind.kind`")?
        {
            console_api::resources::resource::kind::Kind::Known(n) => {
                match console_api::resources::resource::kind::Known::from_i32(n).unwrap() {
                    console_api::resources::resource::kind::Known::Timer => "Timer".to_string(),
                }
            }
            console_api::resources::resource::kind::Kind::Other(s) => s,
        };

        let vis = if resource.is_internal {
            TypeVisibility::Internal
        } else {
            TypeVisibility::Public
        };

        let parent_id = resource.parent_resource_id.map(|id| ResourceId(id.id));

        let concrete_type = resource.concrete_type;

        let location = resource.location.map(Location::try_from).transpose()?;

        Ok(Self {
            id,
            metadata_id,
            vis,
            parent_id,
            kind,
            concrete_type,
            location,
            target: None,
            stats: None,
        })
    }
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

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
enum TypeVisibility {
    Public,
    Internal,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
struct Stats {
    dropped_at: Option<SystemTime>,
    created_at: Option<SystemTime>,
}

impl TryFrom<console_api::resources::Stats> for Stats {
    type Error = anyhow::Error;

    fn try_from(stats: console_api::resources::Stats) -> Result<Self, Self::Error> {
        let console_api::resources::Stats {
            dropped_at,
            created_at,
            attributes: _,
        } = stats;

        let created_at = created_at.map(SystemTime::try_from).transpose()?;
        let dropped_at = dropped_at.map(SystemTime::try_from).transpose()?;

        Ok(Self {
            dropped_at,
            created_at,
        })
    }
}
