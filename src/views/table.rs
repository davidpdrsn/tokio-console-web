use axum_live_view::{html, Html};

pub(crate) trait TableView {
    type Column: std::fmt::Display;
    type Model;
    type Msg;

    fn columns(&self) -> Vec<Self::Column>;

    fn rows(&self) -> Vec<Self::Model>;

    fn render_column(&self, col: &Self::Column, row: &Self::Model) -> Html<Self::Msg>;

    fn row_click_event(&self, row: &Self::Model) -> Self::Msg;

    fn key_event(&self) -> Self::Msg;

    fn row_selected(&self, idx: usize, row: &Self::Model) -> bool;

    fn table_render(&self) -> Html<Self::Msg> {
        let columns = self.columns();
        let rows = self.rows();

        html! {
            <table
                class="resources-table"
                axm-window-keydown={ self.key_event() }
            >
                <thead>
                    <tr>
                        for col in &columns {
                            <th>{ col.to_string() }</th>
                        }
                    </tr>
                </thead>
                <tbody>
                    for (idx, row) in rows.into_iter().enumerate() {
                        <tr
                            axm-click={ self.row_click_event(&row) }
                            class=if self.row_selected(idx, &row) { "row-selected" }
                        >
                            for col in &columns {
                                <td>{ self.render_column(col, &row) }</td>
                            }
                        </tr>
                    }
                </tbody>
            </table>
        }
    }
}
