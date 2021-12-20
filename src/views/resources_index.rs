use crate::cancel_on_drop::CancelOnDrop;
use axum_liveview::{html, pubsub::Bincode, Html, LiveView, ShouldRender};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct ResourcesIndex {
    _token: CancelOnDrop,
    stream_id: Uuid,
}

impl LiveView for ResourcesIndex {
    fn setup(&self, setup: &mut axum_liveview::Setup<Self>) {
        setup.on_broadcast(&msg_topic(self.stream_id), Self::msg);
        setup.on_broadcast("tick", Self::tick);
    }

    fn render(&self) -> Html {
        html! {
            <div>
                "TODO"
            </div>
        }
    }
}

impl ResourcesIndex {
    pub fn new(token: CancelOnDrop, id: Uuid) -> Self {
        Self {
            _token: token,
            stream_id: id,
        }
    }

    async fn msg(self, Bincode(msg): Bincode<Update>) -> ShouldRender<Self> {
        ShouldRender::No(self)
    }

    async fn tick(self) -> Self {
        self
    }
}

pub fn msg_topic(id: Uuid) -> String {
    format!("resources-index/msg/{}", id)
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Update {}

impl TryFrom<console_api::instrument::Update> for Update {
    type Error = anyhow::Error;

    fn try_from(update: console_api::instrument::Update) -> Result<Self, Self::Error> {
        Ok(Self {})
    }
}
