//! Searchable, scrollable application picker for the host-app behavior editor.
//!
//! Makepad's regular `DropDown` renders every option into one popup and does
//! not scroll. This composite widget keeps the interaction bounded by using a
//! `PortalList`; only visible rows are instantiated and drawn.

use makepad_widgets::*;

live_design! {
    link widgets;

    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    AppPickerRow = <Button> {
        width: Fill, height: 38,
        margin: 0,
        padding: {left: 12, right: 12},
        align: {x: 0.0, y: 0.5},
        label_walk: {width: Fill, height: Fit},
        grab_key_focus: false,

        draw_bg: {
            border_radius: 8.0,
            border_size: 1.0,
            color_dither: 0.0,
            color: #11151c,
            color_hover: #242c38,
            color_down: #302719,
            color_focus: #171c25,
            color_disabled: #11151c,
            border_color_1: #1c232d,
            border_color_2: #1c232d,
            border_color_1_hover: #3a4656,
            border_color_2_hover: #3a4656,
            border_color_1_down: #f2aa4c,
            border_color_2_down: #f2aa4c,
            border_color_1_focus: #2a3340,
            border_color_2_focus: #2a3340,
            border_color_1_disabled: #1c232d,
            border_color_2_disabled: #1c232d,
        }
        draw_text: {
            color: #f4f1ea,
            color_hover: #fffaf1,
            color_down: #ffc36b,
            color_focus: #f4f1ea,
            color_disabled: #9297a0,
            text_style: <THEME_FONT_REGULAR> {font_size: 11.5},
        }
    }

    AppPickerSelectedRow = <AppPickerRow> {
        draw_bg: {
            color: #3a2917,
            color_hover: #49331c,
            color_down: #51391f,
            color_focus: #3a2917,
            border_color_1: #f2aa4c,
            border_color_2: #f2aa4c,
            border_color_1_hover: #ffc36b,
            border_color_2_hover: #ffc36b,
        }
        draw_text: {
            color: #ffc36b,
            color_hover: #fffaf1,
        }
    }

    AppPickerEmptyRow = <AppPickerRow> {
        text: "No matching applications",
        enabled: false,
        draw_bg: {
            color: #0d1117,
            color_disabled: #0d1117,
            border_color_1_disabled: #1c232d,
            border_color_2_disabled: #1c232d,
        }
        draw_text: {
            color_disabled: #9297a0,
        }
    }

    pub AppPickerBase = {{AppPicker}} {}

    // A bounded selector intended to replace a long installed-app dropdown.
    pub AppPicker = <AppPickerBase> {
        width: Fill, height: 280,
        flow: Down, spacing: 8,

        app_search = <TextInput> {
            width: Fill, height: 38,
            margin: 0,
            empty_text: "Search applications",
            padding: {left: 12, right: 12},
            draw_bg: {
                border_radius: 9.0,
                border_size: 1.0,
                color_dither: 0.0,
                color: #0d1117,
                color_hover: #11151c,
                color_focus: #0d1117,
                color_down: #0d1117,
                color_empty: #0d1117,
                color_disabled: #0d1117,
                border_color_1: #2a3340,
                border_color_2: #2a3340,
                border_color_1_hover: #3a4656,
                border_color_2_hover: #3a4656,
                border_color_1_focus: #f2aa4c,
                border_color_2_focus: #f2aa4c,
                border_color_1_down: #f2aa4c,
                border_color_2_down: #f2aa4c,
                border_color_1_empty: #2a3340,
                border_color_2_empty: #2a3340,
                border_color_1_disabled: #1c232d,
                border_color_2_disabled: #1c232d,
            }
            draw_text: {
                color: #f4f1ea,
                color_hover: #fffaf1,
                color_focus: #fffaf1,
                color_down: #fffaf1,
                color_empty: #9297a0,
                color_disabled: #9297a0,
                text_style: <THEME_FONT_REGULAR> {font_size: 11.5},
            }
        }

        app_list = <PortalList> {
            width: Fill, height: Fill,
            flow: Down,
            capture_overload: true,
            drag_scrolling: true,
            grab_key_focus: true,
            scroll_bar: <ScrollBar> {
                bar_size: 10.0,
                bar_side_margin: 2.0,
                min_handle_size: 32.0,
                draw_bg: {
                    size: 5.0,
                    border_size: 0.0,
                    border_radius: 2.5,
                    color: #3a4656,
                    color_hover: #9297a0,
                    color_drag: #f2aa4c,
                    border_color: #0000,
                    border_color_hover: #0000,
                    border_color_drag: #0000,
                }
            }

            Row = <AppPickerRow> {}
            SelectedRow = <AppPickerSelectedRow> {}
            EmptyRow = <AppPickerEmptyRow> {}
        }
    }
}

#[derive(Clone, Debug, DefaultNone)]
pub enum AppPickerAction {
    None,
    /// Index into the unfiltered entries supplied to `set_entries`.
    Selected(usize),
}

#[derive(Live, LiveHook, Widget)]
pub struct AppPicker {
    #[deref]
    view: View,
    #[rust]
    entries: Vec<String>,
    /// Indices into `entries`, in the order currently displayed.
    #[rust]
    filtered_indices: Vec<usize>,
    #[rust]
    query: String,
    #[rust]
    selected_item: Option<usize>,
}

impl AppPicker {
    fn rebuild_filter(&mut self) {
        let terms = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();

        self.filtered_indices.clear();
        self.filtered_indices.extend(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, label)| {
                    if terms.is_empty() {
                        return true;
                    }
                    let label = label.to_lowercase();
                    terms.iter().all(|term| label.contains(term))
                })
                .map(|(index, _)| index),
        );
    }

    fn reset_list_position(&self) {
        self.view
            .portal_list(id!(app_list))
            .set_first_id_and_scroll(0, 0.0);
    }

    fn scroll_selection_into_view(&self) {
        let Some(selected_item) = self.selected_item else {
            return;
        };
        let Some(row) = self
            .filtered_indices
            .iter()
            .position(|index| *index == selected_item)
        else {
            return;
        };

        self.view
            .portal_list(id!(app_list))
            .set_first_id_and_scroll(row, 0.0);
    }
}

impl Widget for AppPicker {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let uid = self.widget_uid();
        let actions = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));

        if let Some(query) = self.view.text_input(id!(app_search)).changed(&actions) {
            self.query = query;
            self.rebuild_filter();
            self.reset_list_position();
            self.view.redraw(cx);
        }

        let list = self.view.portal_list(id!(app_list));
        if !list.was_scrolling() {
            for (row_index, row) in list.items_with_actions(&actions) {
                if !row.as_button().clicked(&actions) {
                    continue;
                }
                let Some(entry_index) = self.filtered_indices.get(row_index).copied() else {
                    continue;
                };

                self.selected_item = Some(entry_index);
                self.view.redraw(cx);
                cx.widget_action(uid, &scope.path, AppPickerAction::Selected(entry_index));
                break;
            }
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let list_ref = self.view.portal_list(id!(app_list));

        while let Some(next) = self.view.draw_walk(cx, scope, walk).step() {
            let Some(mut list) = list_ref.borrow_mut_if_eq(&next) else {
                continue;
            };

            let row_count = self.filtered_indices.len().max(1);
            list.set_item_range(cx, 0, row_count);

            while let Some(row_index) = list.next_visible_item(cx) {
                if self.filtered_indices.is_empty() {
                    let row = list.item(cx, row_index, live_id!(EmptyRow));
                    row.draw_all(cx, scope);
                    continue;
                }

                let Some(entry_index) = self.filtered_indices.get(row_index).copied() else {
                    continue;
                };
                let template = if self.selected_item == Some(entry_index) {
                    live_id!(SelectedRow)
                } else {
                    live_id!(Row)
                };
                let row = list.item(cx, row_index, template);
                row.as_button().set_text(cx, &self.entries[entry_index]);
                row.draw_all(cx, scope);
            }
        }

        DrawStep::done()
    }
}

impl AppPickerRef {
    /// Replace all source entries. Emitted indices refer to this vector.
    pub fn set_entries(&self, cx: &mut Cx, entries: Vec<String>) {
        let Some(mut inner) = self.borrow_mut() else {
            return;
        };

        inner.entries = entries;
        if inner
            .selected_item
            .is_some_and(|selected| selected >= inner.entries.len())
        {
            inner.selected_item = None;
        }
        inner.rebuild_filter();
        inner.reset_list_position();
        inner.view.redraw(cx);
    }

    /// Update the highlighted source entry without emitting an action.
    pub fn set_selected_item(&self, cx: &mut Cx, selected_item: Option<usize>) {
        let Some(mut inner) = self.borrow_mut() else {
            return;
        };

        inner.selected_item = selected_item.filter(|index| *index < inner.entries.len());
        inner.scroll_selection_into_view();
        inner.view.redraw(cx);
    }

    pub fn selected_item(&self) -> Option<usize> {
        self.borrow().and_then(|inner| inner.selected_item)
    }

    /// Return the selected source index emitted in this action batch.
    pub fn changed(&self, actions: &Actions) -> Option<usize> {
        let action = actions.find_widget_action(self.widget_uid())?;
        match action.cast() {
            AppPickerAction::Selected(index) => Some(index),
            AppPickerAction::None => None,
        }
    }

    pub fn clear_search(&self, cx: &mut Cx) {
        let Some(mut inner) = self.borrow_mut() else {
            return;
        };

        inner.query.clear();
        inner.view.text_input(id!(app_search)).set_text(cx, "");
        inner.rebuild_filter();
        inner.reset_list_position();
        inner.view.redraw(cx);
    }
}
