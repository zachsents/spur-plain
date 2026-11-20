use std::{ops::Range, time::Duration};

use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::*, px, uniform_list,
};
use jiff::Zoned;
use kit::{ActiveTheme, Button, Icon, StyledExt, TextField, TextFieldEvent, Tick, text};

use crate::{
    actions::{
        AddItem, BeginAdd, BeginRename, BeginRenameList, BeginSetDue, CreateList, NewList,
        RenameItem, RenameList, SelectItem, SelectList, SetDue, Tick as TickItem,
    },
    due::{self, Due},
    tasks::Tasks,
};

/// The app window: the lists in a sidebar, the selected list's items beside it. It only
/// displays them. Every change goes through an action handled app-wide.
pub struct MainWindow {
    tasks: Entity<Tasks>,
    focus_handle: FocusHandle,
    editing: Option<Editing>,
    list_rows: UniformListScrollHandle,
    item_rows: UniformListScrollHandle,
    _observe_tasks: Subscription,
}

/// What a text field in the window is editing.
#[derive(Clone, PartialEq)]
enum Field {
    /// A new item being typed at the end of the list.
    NewItem,
    Title(String),
    Due(String),
    ListName(String),
}

struct Editing {
    field: Field,
    input: Entity<TextField>,
    /// What the decision model made of a due date the parser couldn't read.
    understood: Option<(String, Result<Due, String>)>,
    /// Asks the decision model about the due date once typing pauses.
    reading: Option<Task<()>>,
    _events: Subscription,
    /// Redraws the window as the text changes, for the due date preview.
    _changes: Subscription,
}

impl Editing {
    /// What the typed due date means, once known.
    fn reading(&self, cx: &App) -> Option<Result<Due, String>> {
        let text = self.input.read(cx).text().trim();
        match due::parse(text, Zoned::now().date()) {
            Ok(due) => Some(Ok(due)),
            Err(_) => self
                .understood
                .as_ref()
                .filter(|(understood, _)| understood == text)
                .map(|(_, reading)| reading.clone()),
        }
    }
}

/// A line in the item list.
enum Row {
    Item(String),
    /// The field for a new item.
    Adding,
    Heading(&'static str),
}

impl MainWindow {
    pub fn new(tasks: Entity<Tasks>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            _observe_tasks: cx.observe(&tasks, |_, _, cx| cx.notify()),
            tasks,
            focus_handle,
            editing: None,
            list_rows: UniformListScrollHandle::new(),
            item_rows: UniformListScrollHandle::new(),
        }
    }

    fn begin_add(&mut self, _: &BeginAdd, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_editing(Field::NewItem, String::new(), window, cx);
    }

    fn begin_rename(&mut self, _: &BeginRename, window: &mut Window, cx: &mut Context<Self>) {
        let tasks = self.tasks.read(cx);
        let Some(item) = tasks.selected_item_id().and_then(|id| tasks.item(&id)) else {
            return;
        };
        self.begin_editing(Field::Title(item.id), item.title, window, cx);
    }

    fn begin_set_due(&mut self, _: &BeginSetDue, window: &mut Window, cx: &mut Context<Self>) {
        let tasks = self.tasks.read(cx);
        let Some(item) = tasks.selected_item_id().and_then(|id| tasks.item(&id)) else {
            return;
        };
        let current = item
            .due
            .map(|date| date.strftime("%b %-d").to_string())
            .unwrap_or_default();
        self.begin_editing(Field::Due(item.id), current, window, cx);
    }

    fn begin_rename_list(
        &mut self,
        _: &BeginRenameList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tasks = self.tasks.read(cx);
        let Some(list) = tasks.selected_list_id().and_then(|id| tasks.list(&id)) else {
            return;
        };
        self.begin_editing(Field::ListName(list.id), list.name, window, cx);
    }

    fn create_list(&mut self, _: &CreateList, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(NewList::default().boxed_clone(), cx);
        window.dispatch_action(BeginRenameList.boxed_clone(), cx);
    }

    /// Puts a text field holding `current`, all selected, where `field` is shown. Enter or
    /// clicking away saves; Escape cancels. Text that can't be saved (an empty name or an
    /// unreadable date) beeps and keeps the field open, or is dropped if the field has already
    /// lost focus. A new item's field stays open after Enter for the next one.
    fn begin_editing(
        &mut self,
        field: Field,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| TextField::new(current, window, cx));
        let editing_field = field.clone();
        let events = cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
            let text = input.read(cx).text().trim().to_string();
            let still_typing = input.focus_handle(cx).is_focused(window);
            if let TextFieldEvent::Confirm = event {
                let reading = this
                    .editing
                    .as_ref()
                    .and_then(|editing| editing.reading(cx));
                let valid = match &editing_field {
                    Field::NewItem => true,
                    Field::Title(_) | Field::ListName(_) => !text.is_empty(),
                    // A date the decision model is still reading is sent as typed, and read
                    // again (instantly, from the model's memory) when it arrives.
                    Field::Due(_) => !matches!(reading, Some(Err(_))),
                };
                if !valid && still_typing {
                    window.play_system_bell();
                    return;
                }
                if valid {
                    let tasks = this.tasks.read(cx);
                    let action = match &editing_field {
                        Field::NewItem if text.is_empty() => None,
                        Field::NewItem => Some(
                            AddItem {
                                list: tasks.selected_list_id(),
                                title: text.clone(),
                                due: None,
                            }
                            .boxed_clone(),
                        ),
                        Field::Title(id) => (tasks.item(id).is_some_and(|item| item.title != text))
                            .then(|| {
                                RenameItem {
                                    id: Some(id.clone()),
                                    title: text.clone(),
                                }
                                .boxed_clone()
                            }),
                        Field::Due(id) => Some(
                            SetDue {
                                id: Some(id.clone()),
                                due: if text.is_empty() {
                                    "none".into()
                                } else {
                                    text.clone()
                                },
                            }
                            .boxed_clone(),
                        ),
                        Field::ListName(id) => {
                            (tasks.list(id).is_some_and(|list| list.name != text)).then(|| {
                                RenameList {
                                    id: Some(id.clone()),
                                    name: text.clone(),
                                }
                                .boxed_clone()
                            })
                        }
                    };
                    if let Some(action) = action {
                        window.dispatch_action(action, cx);
                    }
                    // Enter on a new item opens a fresh field for the next one.
                    if editing_field == Field::NewItem && still_typing && !text.is_empty() {
                        this.editing = None;
                        this.begin_editing(Field::NewItem, String::new(), window, cx);
                        return;
                    }
                }
            }
            // Cancelling moves focus away, which also confirms, so only the first event counts.
            if this.editing.take().is_some() {
                window.focus(&this.focus_handle, cx);
                cx.notify();
            }
        });
        self.editing = Some(Editing {
            field,
            _changes: cx.observe(&input, |this, _, cx| {
                this.read_typed_due(cx);
                cx.notify();
            }),
            input,
            understood: None,
            reading: None,
            _events: events,
        });
        cx.notify();
    }

    /// Once typing pauses on a due date the parser can't read, asks the decision model what it
    /// means, so the preview shows it before Enter.
    fn read_typed_due(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = self
            .editing
            .as_mut()
            .filter(|editing| matches!(editing.field, Field::Due(_)))
        else {
            return;
        };
        let text = editing.input.read(cx).text().trim().to_string();
        editing.reading = None;
        if text.is_empty() || editing.reading(cx).is_some() {
            return;
        }
        editing.reading = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            let Ok(understanding) = this.update(cx, |_, cx| due::understand(text.clone(), cx))
            else {
                return;
            };
            let understood = understanding.await;
            this.update(cx, |this, cx| {
                if let Some(editing) = &mut this.editing {
                    editing.understood = Some((text, understood));
                    editing.reading = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The text field editing `field`, if that's being edited.
    fn editor(&self, field: &Field) -> Option<Entity<TextField>> {
        self.editing
            .as_ref()
            .filter(|editing| &editing.field == field)
            .map(|editing| editing.input.clone())
    }

    /// Clicking an item selects it; double-clicking also starts editing its title. Clicking
    /// another row finishes an edit in progress, as clicking away does in Finder.
    fn click_item(
        &mut self,
        id: String,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor(&Field::Title(id.clone())).is_some() {
            return;
        }
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(SelectItem { id }.boxed_clone(), cx);
        if event.click_count() == 2 {
            window.dispatch_action(BeginRename.boxed_clone(), cx);
        }
    }

    fn click_list(
        &mut self,
        id: String,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor(&Field::ListName(id.clone())).is_some() {
            return;
        }
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(SelectList { id }.boxed_clone(), cx);
        if event.click_count() == 2 {
            window.dispatch_action(BeginRenameList.boxed_clone(), cx);
        }
    }

    fn item_rows(&self, cx: &App) -> Vec<Row> {
        let tasks = self.tasks.read(cx);
        let Some(list) = tasks.selected_list_id() else {
            return Vec::new();
        };
        let items = tasks.items(&list);
        let adding = self.editor(&Field::NewItem).is_some();
        let to_do = items.to_do.iter().map(|item| Row::Item(item.id.clone()));
        let done = items.done.iter().map(|item| Row::Item(item.id.clone()));
        let heading = (!items.done.is_empty()).then_some(Row::Heading("Done"));
        to_do
            .chain(adding.then_some(Row::Adding))
            .chain(heading)
            .chain(done)
            .collect()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let count = self.tasks.read(cx).lists().len();
        div()
            .w(px(200.))
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.border)
            .child(kit::titlebar())
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .relative()
                    .child(
                        uniform_list(
                            "lists",
                            count,
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                let lists = this.tasks.read(cx).lists();
                                lists
                                    .get(range)
                                    .unwrap_or_default()
                                    .iter()
                                    .map(|list| this.render_list_row(&list.id, cx))
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.list_rows)
                        .size_full()
                        .px(px(6.)),
                    )
                    .child(kit::scrollbar("lists-scrollbar", &self.list_rows)),
            )
            .child(
                div().flex().p(px(8.)).child(
                    Button::new("new-list", "New List", CreateList)
                        .icon(Icon::ListChecks)
                        .grow(),
                ),
            )
    }

    fn render_list_row(&self, id: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tasks = self.tasks.read(cx);
        let Some(list) = tasks.list(id) else {
            return div().into_any_element();
        };
        let selected = tasks.selected_list_id().as_deref() == Some(id);
        let left = tasks.items(id).to_do.len();
        let editor = self.editor(&Field::ListName(id.to_string()));
        let id = id.to_string();
        kit::row(format!("list-{id}"), selected, cx)
            .on_click(cx.listener(move |this, event, window, cx| {
                this.click_list(id.clone(), event, window, cx)
            }))
            .child(div().flex_1().min_w(px(0.)).map(|this| match editor {
                Some(input) => this.child(input),
                None => this.truncate().child(list.name.clone()),
            }))
            .when(left > 0, |this| {
                this.child(
                    div()
                        .tabular_nums()
                        .text_color(theme.text_muted)
                        .child(left.to_string()),
                )
            })
            .into_any_element()
    }

    fn render_items(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tasks = self.tasks.read(cx);
        let list = tasks.selected_list_id().and_then(|id| tasks.list(&id));
        let rows = self.item_rows(cx).len();
        let pane = div().flex_1().min_w(px(0.)).flex().flex_col().child(
            kit::titlebar()
                .justify_center()
                .text_color(theme.text_muted)
                .child(
                    list.as_ref()
                        .map(|list| list.name.clone())
                        .unwrap_or_default(),
                ),
        );
        let Some(_) = list else {
            return pane.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_faint)
                    .child("No lists"),
            );
        };
        let body = if rows == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_faint)
                .child("Nothing to do")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h(px(0.))
                .relative()
                .child(
                    uniform_list(
                        "items",
                        rows,
                        cx.processor(|this, range: Range<usize>, window, cx| {
                            this.item_rows(cx)
                                .drain(range)
                                .map(|row| this.render_item_row(row, window, cx))
                                .collect()
                        }),
                    )
                    .track_scroll(&self.item_rows)
                    .size_full()
                    .px(px(6.)),
                )
                .child(kit::scrollbar("items-scrollbar", &self.item_rows))
                .into_any_element()
        };
        let _ = window;
        pane.child(body).child(
            div().flex().p(px(8.)).child(
                Button::new("new-item", "New Item", BeginAdd)
                    .icon(Icon::Plus)
                    .grow(),
            ),
        )
    }

    fn render_item_row(&self, row: Row, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let id = match row {
            Row::Heading(title) => {
                return div()
                    .w_full()
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_end()
                    .pb(px(4.))
                    .text_size(text::SMALL)
                    .text_color(theme.text_muted)
                    .child(title)
                    .into_any_element();
            }
            Row::Adding => {
                let input = self.editor(&Field::NewItem);
                return div()
                    .w_full()
                    .h(px(28.))
                    .pr(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.text_faint)
                            .child(kit::icon(Icon::Circle)),
                    )
                    .child(div().flex_1().min_w(px(0.)).children(input))
                    .into_any_element();
            }
            Row::Item(id) => id,
        };
        let tasks = self.tasks.read(cx);
        let Some(item) = tasks.item(&id) else {
            return div().into_any_element();
        };
        let selected = tasks.selected_item_id().as_deref() == Some(id.as_str());
        let today = Zoned::now().date();
        let title_editor = self.editor(&Field::Title(id.clone()));
        let due_editor = self.editor(&Field::Due(id.clone()));
        let label = if item.done { "Untick" } else { "Tick" };
        let tick = Tick::new(
            format!("tick-{id}"),
            item.done,
            label,
            TickItem {
                id: Some(id.clone()),
                done: None,
            },
            TickItem::default(),
        )
        .selected(selected);
        // While editing, the due column previews what the typed date means.
        let due: Option<AnyElement> = match due_editor {
            Some(input) => {
                let editing = self.editing.as_ref().expect("an editor means editing");
                let (preview, color) = match editing.reading(cx) {
                    Some(Ok(Some(date))) => (date.strftime("%a, %b %-d").to_string(), theme.text),
                    Some(Ok(None)) => ("No date".to_string(), theme.text_muted),
                    Some(Err(problem)) => (problem, theme.text_muted),
                    None if editing.reading.is_some() => ("…".to_string(), theme.text_muted),
                    None => (String::new(), theme.text_muted),
                };
                Some(
                    div()
                        .w(px(220.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().w(px(96.)).flex_none().child(input))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(text::SMALL)
                                .text_color(color)
                                .child(preview),
                        )
                        .into_any_element(),
                )
            }
            None => item.due.map(|date| {
                let overdue = !item.done && date < today;
                div()
                    .id(format!("due-{id}"))
                    .relative()
                    .px(px(6.))
                    .rounded(px(6.))
                    .tabular_nums()
                    .text_color(if overdue {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface))
                    .on_click({
                        let id = id.clone();
                        move |_, window, cx| {
                            window.dispatch_action(SelectItem { id: id.clone() }.boxed_clone(), cx);
                            window.dispatch_action(BeginSetDue.boxed_clone(), cx);
                        }
                    })
                    .child(due::describe(date, today))
                    .children(
                        selected
                            .then(|| kit::reveal_shortcut(&BeginSetDue, window, cx))
                            .flatten(),
                    )
                    .into_any_element()
            }),
        };
        let row_id = id.clone();
        kit::row(format!("item-{id}"), selected, cx)
            .pl(px(0.))
            .gap(px(2.))
            .on_click(cx.listener(move |this, event, window, cx| {
                this.click_item(row_id.clone(), event, window, cx)
            }))
            .child(tick)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .when(item.done, |this| this.text_color(theme.text_muted))
                    .map(|this| match title_editor {
                        Some(input) => this.child(input),
                        None => this.truncate().child(item.title.clone()),
                    }),
            )
            .children(due)
            .into_any_element()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit::root(cx)
            .flex_row()
            .track_focus(&self.focus_handle)
            .key_context("Tasks")
            .on_action(cx.listener(Self::begin_add))
            .on_action(cx.listener(Self::begin_rename))
            .on_action(cx.listener(Self::begin_set_due))
            .on_action(cx.listener(Self::begin_rename_list))
            .on_action(cx.listener(Self::create_list))
            .child(self.render_sidebar(cx))
            .child(self.render_items(window, cx))
    }
}
