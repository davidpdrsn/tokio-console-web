use std::ops::Deref;

use crate::{routes::ConsoleAddr, watch_stream::Location};
use axum::{
    async_trait,
    http::{HeaderMap, Uri},
};
use axum_live_view::{
    event_data::EventData,
    html,
    live_view::{Updated, ViewHandle},
    Html, LiveView,
};

pub mod resources_index;
pub mod tasks_index;

mod layout;
mod table;
mod table_view_keybinds;

pub use self::layout::{Layout, TaskResourceLayout};

impl Location {
    fn render<T>(&self) -> Html<T> {
        html! {
            { &self.file } ":" { self.line } ":" { self.column }
        }
    }
}

pub struct ConnectionFailed {
    pub addr: ConsoleAddr,
    pub err: anyhow::Error,
}

#[async_trait]
impl LiveView for ConnectionFailed {
    type Message = ();
    type Error = anyhow::Error;

    async fn mount(
        &mut self,
        _uri: Uri,
        _request_headers: &HeaderMap,
        _handle: ViewHandle<Self::Message>,
    ) -> Result<(), Self::Error> {
        anyhow::bail!("reconnecting...")
    }

    async fn update(
        mut self,
        _msg: Self::Message,
        _data: Option<EventData>,
    ) -> Result<Updated<Self>, Self::Error> {
        anyhow::bail!("reconnecting...")
    }

    fn render(&self) -> Html<Self::Message> {
        html! {
            <div>
                "Connection failed: " { &self.err }
            </div>
        }
    }
}

enum StateRef<'a, T> {
    BorrowedFromWatch(tokio::sync::watch::Ref<'a, T>),
    Ref(&'a T),
}

impl<'a, T> Deref for StateRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            StateRef::BorrowedFromWatch(r) => &**r,
            StateRef::Ref(r) => *r,
        }
    }
}
