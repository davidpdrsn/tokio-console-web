use axum_live_view::{event_data::EventData, html, Html};

#[derive(Default)]
pub(crate) struct TableViewKeybinds {
    selected_idx: Option<usize>,
    show_key_binds: bool,
}

impl TableViewKeybinds {
    pub(crate) fn selected_idx(&self) -> Option<usize> {
        self.selected_idx
    }

    pub(crate) fn clamp_selected_idx(&mut self, new_max: usize) {
        if let Some(idx) = self.selected_idx.as_mut() {
            *idx = std::cmp::min(new_max - 1, *idx);
        }
    }

    pub(crate) fn update(&mut self, data: Option<&EventData>) -> Option<TableViewKeybindsUpdate> {
        let data = data.unwrap();
        let key = data.as_key().unwrap().key();

        match key {
            "k" => {
                if let Some(idx) = self.selected_idx.as_mut() {
                    if *idx != 0 {
                        *idx -= 1;
                    }
                }

                None
            }
            "j" => {
                if let Some(idx) = self.selected_idx.as_mut() {
                    *idx += 1;
                } else {
                    self.selected_idx = Some(0);
                }

                None
            }
            "Enter" => self.selected_idx.map(TableViewKeybindsUpdate::Selected),
            " " => Some(TableViewKeybindsUpdate::TogglePlayPause),
            "?" => {
                self.show_key_binds = !self.show_key_binds;
                None
            }
            "t" => Some(TableViewKeybindsUpdate::GotoTasks),
            "r" => Some(TableViewKeybindsUpdate::GotoResources),
            _ => None,
        }
    }

    pub(crate) fn help<T>(&self) -> Html<T> {
        if self.show_key_binds {
            html! {
                <div class="keybinds">
                    "Key binds<br>"
                    "j/k: down/up<br>"
                    "space: play/pause<br>"
                    "enter: open<br>"
                    "t: goto tasks"
                    "r: goto resources"
                    "?: show/hide keybinds"
                </div>
            }
        } else {
            html! {}
        }
    }
}

pub(crate) enum TableViewKeybindsUpdate {
    TogglePlayPause,
    Selected(usize),
    GotoTasks,
    GotoResources,
}
