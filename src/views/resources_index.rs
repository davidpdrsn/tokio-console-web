use super::{Location, MetaId, Metadata};
use crate::{cancel_on_drop::CancelOnDrop, routes::ConsoleAddr};
use anyhow::Context;
use axum::Json;
use axum_liveview::{html, pubsub::Bincode, Html, LiveView, RenderResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, SystemTime},
};
use uuid::Uuid;

pub struct ResourcesIndex {
    _token: CancelOnDrop,
    stream_id: Uuid,
    paused: bool,
    resources: BTreeMap<ResourceId, Resource>,
    metadata: HashMap<MetaId, Metadata>,
    addr: ConsoleAddr,
}

impl LiveView for ResourcesIndex {
    fn setup(&self, setup: &mut axum_liveview::Setup<Self>) {
        setup.on_broadcast(&msg_topic(self.stream_id), Self::msg);
        setup.on_broadcast("tick", Self::tick);
        setup.on("toggle-play-pause", Self::toggle_play_pause);
        setup.on("row-click", Self::row_click);
    }

    fn render(&self) -> Html {
        html! {
            <div>
                if self.resources.is_empty() {
                    "Loading..."
                } else {
                    <div>
                        if self.paused {
                            <button live-click="toggle-play-pause">"Play"</button>
                        } else {
                            <button live-click="toggle-play-pause">"Pause"</button>
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
                            for resource in self.resources.values() {
                                { resource.render_as_table_row() }
                            }
                        </tbody>
                    </table>
                }
            </div>
        }
    }
}

impl ResourcesIndex {
    pub fn new(token: CancelOnDrop, id: Uuid, addr: ConsoleAddr) -> Self {
        Self {
            _token: token,
            stream_id: id,
            paused: false,
            resources: Default::default(),
            metadata: Default::default(),
            addr,
        }
    }

    async fn msg(mut self, Bincode(msg): Bincode<Update>) -> RenderResult<Self> {
        let Update {
            new_resources,
            new_metadata,
            stats_update,
        } = msg;

        for resources in new_resources {
            self.resources.insert(resources.id, resources);
        }

        self.metadata.extend(new_metadata);

        for (id, stats) in stats_update {
            if let Some(resource) = self.resources.get_mut(&id) {
                resource.stats = Some(stats);
            }
        }

        for resource in self.resources.values_mut() {
            if let Some(metadata) = self.metadata.get(&resource.metadata_id) {
                resource.target = Some(metadata.target.clone());
            }
        }

        RenderResult::dont_render(self)
    }

    async fn tick(mut self) -> RenderResult<Self> {
        if self.paused {
            RenderResult::dont_render(self)
        } else {
            self.reap();
            RenderResult::render(self)
        }
    }

    async fn row_click(self, Json(data): axum::Json<Value>) -> RenderResult<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct Data {
            resource_id: String,
        }

        let data = serde_json::from_value::<Data>(data).unwrap();

        let uri = format!(
            "/console/{}/{}/resources/{}",
            self.addr.ip, self.addr.port, data.resource_id
        )
        .parse()
        .expect("invalid URI");

        RenderResult::navigate_to(uri)
    }

    async fn toggle_play_pause(mut self) -> Self {
        self.paused = !self.paused;
        self
    }

    fn reap(&mut self) {
        self.resources.retain(|_id, resource| {
            if let Some(stats) = &resource.stats {
                if let Some(dropped_at) = stats.dropped_at {
                    dropped_at.elapsed().unwrap() < Duration::from_secs(5)
                } else {
                    true
                }
            } else {
                true
            }
        })
    }
}

pub fn msg_topic(id: Uuid) -> String {
    format!("resources-index/msg/{}", id)
}

#[derive(Deserialize, Serialize, Debug)]
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

#[derive(Deserialize, Serialize, Debug)]
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
struct ResourceId(u64);

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
    fn render_as_table_row(&self) -> Html {
        html! {
            <tr live-click="row-click" live-data-resource-id={ self.id.0 }>
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

#[derive(Deserialize, Serialize, Debug)]
enum TypeVisibility {
    Public,
    Internal,
}

#[derive(Deserialize, Serialize, Debug)]
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
