//! The list widget: one list as people use it wherever it's shown, written in A2UI's basic
//! catalog so any A2UI renderer draws it. A tap is an action naming the tool it calls, with the
//! tool's arguments as its context: the tick boxes call `tick` and the field's button calls
//! `add_item`.

use jiff::civil::Date;
use kit::{Icon, Surface, svg_icon};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{due, tasks::Tasks};

/// Show a list as a widget people can use right where it's shown: what's left to do, each with
/// a box that ticks it off, the items ticked off most recently, and a field to add one. It
/// stays live as the list changes. Defaults to the selected list.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowList {
    /// Which list. Defaults to the selected one.
    #[serde(default)]
    pub list: Option<String>,
}

/// How many done items the widget shows, most recent first.
const SHOWN_DONE: usize = 5;

/// How many items the summary names before counting the rest.
const NAMED: usize = 10;

pub fn draw(tasks: &Tasks, arguments: &ShowList, today: Date) -> Result<Surface, String> {
    let id = match &arguments.list {
        Some(id) => id.clone(),
        None => tasks.selected_list_id().ok_or("there are no lists")?,
    };
    let list = tasks
        .list(&id)
        .ok_or_else(|| format!("there's no list with id {id}"))?;
    let items = tasks.items(&list.id);
    let left = match items.to_do.len() {
        0 => "Nothing to do".to_string(),
        count => format!("{count} to do"),
    };
    let to_do: Vec<Value> = items
        .to_do
        .iter()
        .map(|item| {
            let due = item
                .due
                .map(|due| due::describe(due, today))
                .unwrap_or_default();
            json!({ "id": item.id, "title": item.title, "due": due })
        })
        .collect();
    let done: Vec<Value> = items
        .done
        .iter()
        .take(SHOWN_DONE)
        .map(|item| json!({ "id": item.id, "title": item.title }))
        .collect();
    let data = json!({
        "list": list.id,
        "name": list.name,
        "left": left,
        "to_do": to_do,
        "done_label": if done.is_empty() { "" } else { "Done" },
        "done": done,
        "draft": "",
    });
    Ok(Surface {
        summary: summary(&list.name, &items, today),
        arguments: json!({ "list": list.id }),
        components: components(),
        data,
    })
}

/// "Groceries: Milk, Eggs (due Friday) and Bread to do; 2 done."
fn summary(name: &str, items: &crate::tasks::Items, today: Date) -> String {
    let mut named: Vec<String> = items
        .to_do
        .iter()
        .take(NAMED)
        .map(|item| match item.due {
            Some(due) => format!("{} (due {})", item.title, due::describe(due, today)),
            None => item.title.clone(),
        })
        .collect();
    let more = items.to_do.len().saturating_sub(NAMED);
    if more > 0 {
        named.push(format!("{more} more"));
    }
    let to_do = match named.split_last() {
        None => "nothing to do".to_string(),
        Some((only, [])) => format!("{only} to do"),
        Some((last, rest)) => format!("{} and {last} to do", rest.join(", ")),
    };
    match items.done.len() {
        0 => format!("{name}: {to_do}."),
        done => format!("{name}: {to_do}; {done} done."),
    }
}

/// The widget's components. They don't change; the data they bind to does.
fn components() -> Vec<Value> {
    vec![
        json!({
            "id": "root",
            "component": "Column",
            "children": ["header", "to_do_list", "done_label", "done_list", "add_row"],
        }),
        json!({
            "id": "header",
            "component": "Row",
            "justify": "spaceBetween",
            "align": "center",
            "children": ["name", "left"],
        }),
        json!({ "id": "name", "component": "Text", "text": { "@path": "/name" } }),
        json!({ "id": "left", "component": "Text", "variant": "caption", "text": { "@path": "/left" } }),
        json!({
            "id": "to_do_list",
            "component": "List",
            "children": { "path": "/to_do", "componentId": "to_do_row" },
        }),
        json!({
            "id": "to_do_row",
            "component": "Row",
            "align": "center",
            "children": ["tick", "title", "due"],
        }),
        json!({
            "id": "tick",
            "component": "Button",
            "variant": "borderless",
            "child": "tick_mark",
            "action": { "event": { "name": "tick", "context": { "id": { "@path": "id" }, "done": true } } },
        }),
        json!({ "id": "tick_mark", "component": "Icon", "name": svg_icon(Icon::Circle) }),
        json!({ "id": "title", "component": "Text", "text": { "@path": "title" }, "weight": 1 }),
        json!({ "id": "due", "component": "Text", "variant": "caption", "text": { "@path": "due" } }),
        json!({ "id": "done_label", "component": "Text", "variant": "caption", "text": { "@path": "/done_label" } }),
        json!({
            "id": "done_list",
            "component": "List",
            "children": { "path": "/done", "componentId": "done_row" },
        }),
        json!({
            "id": "done_row",
            "component": "Row",
            "align": "center",
            "children": ["untick", "done_title"],
        }),
        json!({
            "id": "untick",
            "component": "Button",
            "variant": "borderless",
            "child": "done_mark",
            "action": { "event": { "name": "tick", "context": { "id": { "@path": "id" }, "done": false } } },
        }),
        json!({ "id": "done_mark", "component": "Icon", "name": svg_icon(Icon::CheckCircle) }),
        json!({
            "id": "done_title",
            "component": "Text",
            "variant": "caption",
            "text": { "@path": "title" },
            "weight": 1,
        }),
        json!({
            "id": "add_row",
            "component": "Row",
            "align": "center",
            "children": ["draft", "add"],
        }),
        json!({
            "id": "draft",
            "component": "TextField",
            "label": "",
            "placeholder": "New item",
            "value": { "@path": "/draft" },
            "variant": "shortText",
            "weight": 1,
        }),
        json!({
            "id": "add",
            "component": "Button",
            "variant": "primary",
            "child": "add_label",
            "action": {
                "event": {
                    "name": "add_item",
                    "context": { "list": { "@path": "/list" }, "title": { "@path": "/draft" } },
                },
            },
        }),
        json!({ "id": "add_label", "component": "Text", "text": "Add" }),
    ]
}
