use super::{FILTERS, ModelMenu, Provider};
use gpui::{
    AnyElement, App, Context, EntityInputHandler, FocusHandle, Focusable, Keystroke, Window,
};
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ModelKey(pub Provider, pub String);

pub(super) enum ResultItem {
    Element(AnyElement),
    Model(ResultRow),
}

pub(super) struct ResultRow {
    pub key: ModelKey,
    pub name: String,
    pub detail: Option<String>,
    pub available: bool,
    pub favorite: bool,
    pub active: bool,
    pub handoff: bool,
}

#[derive(Clone)]
pub(super) struct ResultFocus {
    pub key: ModelKey,
    pub choice: FocusHandle,
    pub star: FocusHandle,
    pub available: bool,
    pub favorite: bool,
    pub list_index: usize,
}

#[derive(Clone)]
pub(super) struct EffortFocus {
    pub provider: Provider,
    pub value: String,
    pub focus: FocusHandle,
}

pub(super) struct Keyboard {
    pub filters: [FocusHandle; 3],
    pub rows: Vec<ResultFocus>,
    pub efforts: Vec<EffortFocus>,
    pub current: Option<ModelKey>,
}

impl Keyboard {
    pub fn new(app: &App) -> Self {
        Self {
            filters: std::array::from_fn(|_| app.focus_handle()),
            rows: Vec::new(),
            efforts: Vec::new(),
            current: None,
        }
    }

    pub fn rebuild_results(
        &mut self,
        items: &[ResultItem],
        search: &FocusHandle,
        window: &mut Window,
        app: &App,
    ) -> Option<ModelKey> {
        let old_rows = std::mem::take(&mut self.rows);
        let focused = old_rows.iter().enumerate().find_map(|(index, row)| {
            if row.star.is_focused(window) {
                Some((index, &row.key, true))
            } else if row.choice.is_focused(window) {
                Some((index, &row.key, false))
            } else {
                None
            }
        });
        let previous: HashMap<_, _> = old_rows.iter().map(|row| (&row.key, row)).collect();
        let mut active = None;
        for (index, item) in items.iter().enumerate() {
            let ResultItem::Model(row) = item else {
                continue;
            };
            if row.active && row.available {
                active = Some(row.key.clone());
            }
            let (choice, star) = previous.get(&row.key).map_or_else(
                || (app.focus_handle(), app.focus_handle()),
                |old| (old.choice.clone(), old.star.clone()),
            );
            self.rows.push(ResultFocus {
                key: row.key.clone(),
                choice,
                star,
                available: row.available,
                favorite: row.favorite,
                list_index: index,
            });
        }
        if let Some((index, key, star)) = focused {
            if let Some(current) = self
                .rows
                .iter()
                .find(|row| &row.key == key && (star || row.available))
            {
                self.current = Some(current.key.clone());
            } else if star
                && let Some(next) = self.rows.get(index.min(self.rows.len().saturating_sub(1)))
            {
                self.current = Some(next.key.clone());
                window.focus(&next.star);
            } else {
                self.current = None;
                window.focus(search);
            }
        }
        self.current
            .clone()
            .filter(|key| self.rows.iter().any(|row| row.available && &row.key == key))
            .or(active)
            .or_else(|| {
                self.rows
                    .iter()
                    .find(|row| row.available)
                    .map(|row| row.key.clone())
            })
    }
}

fn adjacent(key: &str, index: Option<usize>, length: usize) -> Option<usize> {
    if length == 0 {
        return None;
    }
    match key {
        "down" | "right" => Some(index.map_or(0, |index| (index + 1) % length)),
        "up" | "left" => Some(index.map_or(length - 1, |index| (index + length - 1) % length)),
        "home" => Some(0),
        "end" => Some(length - 1),
        _ => None,
    }
}

impl ModelMenu {
    pub(super) fn handle_menu_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search = self.query.read(cx).focus_handle(cx);
        let search_focused = search.is_focused(window);
        if search_focused
            && self.query.update(cx, |input, cx| {
                input.marked_text_range(window, cx).is_some()
            })
        {
            return;
        }
        if self.dispatch_menu_key(keystroke, &search, search_focused, window, cx) {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn dispatch_menu_key(
        &mut self,
        keystroke: &Keystroke,
        search: &FocusHandle,
        search_focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = keystroke.key.as_str();
        if key == "escape" {
            let _ = self
                .owner
                .update(cx, |library, cx| library.close_model_menu(true, window, cx));
            return true;
        }
        let modifiers = keystroke.modifiers;
        if modifiers.control
            && !(modifiers.alt || modifiers.shift || modifiers.platform)
            && key.len() == 1
            && let Ok(number @ 1..=9) = key.parse::<usize>()
            && let Some(row) = self
                .keyboard
                .rows
                .iter()
                .filter(|row| row.available)
                .nth(number - 1)
                .cloned()
        {
            self.focus_result(&row, false, window, cx);
            self.choose_key(&row.key, cx);
            return true;
        }
        let activate = matches!(key, "enter" | "space");
        if let Some(index) = self
            .keyboard
            .filters
            .iter()
            .position(|focus| focus.is_focused(window))
        {
            if activate {
                self.switch_scope(FILTERS[index], window, cx);
                return true;
            }
            if key == "right" {
                window.focus(search);
                return true;
            }
            if matches!(key, "up" | "down" | "home" | "end")
                && let Some(next) = adjacent(key, Some(index), self.keyboard.filters.len())
            {
                window.focus(&self.keyboard.filters[next]);
                return true;
            }
            return false;
        }
        if let Some(index) = self
            .keyboard
            .efforts
            .iter()
            .position(|effort| effort.focus.is_focused(window))
            && let Some(next) = if activate {
                Some(index)
            } else {
                adjacent(key, Some(index), self.keyboard.efforts.len())
            }
        {
            let target = self.keyboard.efforts[next].clone();
            window.focus(&target.focus);
            self.choose_effort(target.provider, &target.value, cx);
            return true;
        }
        if activate
            && let Some(row) = self
                .keyboard
                .rows
                .iter()
                .find(|row| {
                    row.star.is_focused(window) || (row.available && row.choice.is_focused(window))
                })
                .cloned()
        {
            if row.star.is_focused(window) {
                self.set_favorite(&row.key, !row.favorite, cx);
            } else {
                self.choose_key(&row.key, cx);
            }
            return true;
        }
        self.handle_result_key(key, search_focused, window, cx)
    }

    fn handle_result_key(
        &mut self,
        key: &str,
        search_focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let choices: Vec<_> = self
            .keyboard
            .rows
            .iter()
            .filter(|row| row.available)
            .collect();
        let index = choices
            .iter()
            .position(|row| row.choice.is_focused(window) || row.star.is_focused(window));
        if search_focused && key == "enter" && !self.query.read(cx).value().trim().is_empty() {
            let Some(first) = choices.first().map(|row| (**row).clone()) else {
                return false;
            };
            let select = choices.len() == 1;
            self.focus_result(&first, false, window, cx);
            if select {
                self.choose_key(&first.key, cx);
            }
            return true;
        }
        if (search_focused && matches!(key, "up" | "down")
            || index.is_some() && matches!(key, "up" | "down" | "home" | "end"))
            && let Some(next) = adjacent(key, index, choices.len())
        {
            let row = (*choices[next]).clone();
            self.focus_result(&row, false, window, cx);
            return true;
        }
        if let Some(index) = index {
            let row = (*choices[index]).clone();
            if key == "right" && row.choice.is_focused(window) {
                self.focus_result(&row, true, window, cx);
                return true;
            }
            if key == "left" && row.star.is_focused(window) {
                self.focus_result(&row, false, window, cx);
                return true;
            }
        }
        false
    }

    fn focus_result(
        &mut self,
        row: &ResultFocus,
        star: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.keyboard.current = Some(row.key.clone());
        self.scroll.scroll_to_item(row.list_index);
        window.focus(if star { &row.star } else { &row.choice });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::adjacent;

    #[test]
    fn wraps_results_and_preserves_native_search_keys() {
        assert_eq!(adjacent("down", None, 3), Some(0));
        assert_eq!(adjacent("up", None, 3), Some(2));
        assert_eq!(adjacent("down", Some(2), 3), Some(0));
        assert_eq!(adjacent("up", Some(0), 3), Some(2));
        assert_eq!(adjacent("home", Some(2), 3), Some(0));
        assert_eq!(adjacent("end", Some(0), 3), Some(2));
        assert_eq!(adjacent("space", Some(0), 3), None);
        assert_eq!(adjacent("down", None, 0), None);
    }
}
