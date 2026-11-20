//! The lists and their items, kept in the app's store so every device's copy merges with the
//! others'. Commands that take an optional id act on the selection when none is given. Errors
//! are worded as replies to agents.

use std::path::PathBuf;

use jiff::{Timestamp, civil::Date};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use store::{Applied, Entity, Store};

use crate::due::{self, Due};

/// The list there is before any other, with the same id on every device, so devices that each
/// start one end up with one.
pub const FIRST_LIST: &str = "todo";

#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub id: String,
    pub name: String,
    position: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub list: String,
    pub title: String,
    pub done: bool,
    /// When it was ticked off, in milliseconds since the Unix epoch.
    pub done_at: Option<i64>,
    pub due: Option<Date>,
    position: String,
}

/// A list's items: what's left to do in order, then what's done, most recently done first.
pub struct Items {
    pub to_do: Vec<Item>,
    pub done: Vec<Item>,
}

impl Items {
    /// Every item in the order the window lists them.
    pub fn all(&self) -> impl Iterator<Item = &Item> {
        self.to_do.iter().chain(&self.done)
    }
}

/// What's selected, which stays on this device.
#[derive(Default, Serialize, Deserialize)]
pub struct Selection {
    list: Option<String>,
    item: Option<String>,
}

pub struct Tasks {
    store: Store,
    selection: Selection,
    /// Changes that took effect and haven't been told to watching agents yet.
    unannounced: Vec<Applied>,
    /// Where the lists are shared with the person's other devices, and what last went wrong.
    sync: Option<(PathBuf, Option<String>)>,
}

impl Tasks {
    pub fn new(store: Store, selection: Selection) -> Self {
        let mut tasks = Self {
            store,
            selection,
            unannounced: Vec::new(),
            sync: None,
        };
        if tasks.lists().is_empty() {
            let fields = object(
                json!({ "name": "To Do", "position": between(None, None), "deleted": false }),
            );
            tasks.write("list", FIRST_LIST, fields).ok();
        }
        tasks
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Changes made or merged since this was last asked, for telling watching agents.
    pub fn take_unannounced(&mut self) -> Vec<Applied> {
        std::mem::take(&mut self.unannounced)
    }

    /// Notes changes that arrived from other devices.
    pub fn merged(&mut self, applied: Vec<Applied>) {
        self.unannounced.extend(applied);
    }

    /// Notes that the lists are shared through `folder`.
    pub fn syncing(&mut self, folder: PathBuf) {
        self.sync = Some((folder, None));
    }

    /// Notes what went wrong sharing the lists, or that nothing did.
    pub fn sync_problem(&mut self, problem: Option<String>) {
        if let Some((_, current)) = &mut self.sync {
            *current = problem;
        }
    }

    fn write(&mut self, kind: &str, id: &str, fields: Map<String, Value>) -> Result<(), String> {
        let applied = self.store.write(kind, id, fields)?;
        self.unannounced.push(applied);
        Ok(())
    }

    pub fn lists(&self) -> Vec<List> {
        let mut lists: Vec<List> = self
            .store
            .entities("list")
            .filter(|(_, entity)| !entity.deleted())
            .map(|(id, entity)| list(id, entity))
            .collect();
        lists.sort_by(|a, b| (&a.position, &a.id).cmp(&(&b.position, &b.id)));
        lists
    }

    pub fn list(&self, id: &str) -> Option<List> {
        self.store
            .entity("list", id)
            .filter(|entity| !entity.deleted())
            .map(|entity| list(id, entity))
    }

    pub fn item(&self, id: &str) -> Option<Item> {
        let item = self
            .store
            .entity("item", id)
            .filter(|entity| !entity.deleted())
            .map(|entity| item(id, entity))?;
        self.list(&item.list).map(|_| item)
    }

    pub fn items(&self, list: &str) -> Items {
        let (mut done, mut to_do): (Vec<Item>, Vec<Item>) = self
            .store
            .entities("item")
            .filter(|(_, entity)| !entity.deleted() && entity.str("list") == Some(list))
            .map(|(id, entity)| item(id, entity))
            .partition(|item| item.done);
        to_do.sort_by(|a, b| (&a.position, &a.id).cmp(&(&b.position, &b.id)));
        done.sort_by(|a, b| b.done_at.cmp(&a.done_at).then_with(|| a.id.cmp(&b.id)));
        Items { to_do, done }
    }

    /// The selected list, falling back to the first when nothing valid is selected.
    pub fn selected_list_id(&self) -> Option<String> {
        let lists = self.lists();
        self.selection
            .list
            .clone()
            .filter(|id| lists.iter().any(|list| &list.id == id))
            .or_else(|| lists.first().map(|list| list.id.clone()))
    }

    /// The selected item, if it's in the selected list.
    pub fn selected_item_id(&self) -> Option<String> {
        let list = self.selected_list_id()?;
        self.selection
            .item
            .clone()
            .filter(|id| self.item(id).is_some_and(|item| item.list == list))
    }

    fn list_id(&self, id: Option<&str>) -> Result<String, String> {
        match id {
            Some(id) => self
                .list(id)
                .map(|list| list.id)
                .ok_or_else(|| format!("there's no list with id {id}")),
            None => self
                .selected_list_id()
                .ok_or_else(|| "there are no lists".to_string()),
        }
    }

    fn item_id(&self, id: Option<&str>) -> Result<Item, String> {
        match id {
            Some(id) => self
                .item(id)
                .ok_or_else(|| format!("there's no item with id {id}")),
            None => self
                .selected_item_id()
                .and_then(|id| self.item(&id))
                .ok_or_else(|| "no item is selected".to_string()),
        }
    }

    pub fn select_list(&mut self, id: &str) -> Result<(), String> {
        let id = self.list_id(Some(id))?;
        if self.selection.list.as_deref() != Some(id.as_str()) {
            self.selection.item = None;
        }
        self.selection.list = Some(id);
        Ok(())
    }

    /// Moves the list selection up (negative) or down (positive), stopping at the ends.
    pub fn select_list_by_offset(&mut self, offset: isize) {
        let lists = self.lists();
        let target = self
            .selected_list_id()
            .and_then(|id| lists.iter().position(|list| list.id == id))
            .and_then(|index| index.checked_add_signed(offset))
            .and_then(|index| lists.get(index));
        if let Some(target) = target {
            self.selection.list = Some(target.id.clone());
            self.selection.item = None;
        }
    }

    pub fn select_item(&mut self, id: &str) -> Result<(), String> {
        let item = self.item_id(Some(id))?;
        self.selection.list = Some(item.list);
        self.selection.item = Some(item.id);
        Ok(())
    }

    /// Moves the item selection up (negative) or down (positive) through the selected list,
    /// stopping at the ends. With nothing selected, it starts from the top or bottom.
    pub fn select_item_by_offset(&mut self, offset: isize) {
        let Some(list) = self.selected_list_id() else {
            return;
        };
        let ids: Vec<String> = self
            .items(&list)
            .all()
            .map(|item| item.id.clone())
            .collect();
        let current = self
            .selected_item_id()
            .and_then(|id| ids.iter().position(|existing| *existing == id));
        let target = match current {
            Some(index) => index
                .checked_add_signed(offset)
                .and_then(|index| ids.get(index)),
            None if offset > 0 => ids.first(),
            None => ids.last(),
        };
        if let Some(target) = target {
            self.selection.item = Some(target.clone());
        }
    }

    pub fn new_list(&mut self, name: Option<&str>) -> Result<String, String> {
        let name = self.unique_name(name.map_or("List 1", str::trim), None);
        if name.is_empty() {
            return Err("a name can't be empty".into());
        }
        let last = self.lists().last().map(|list| list.position.clone());
        let id = Store::new_id();
        let fields = object(json!({
            "name": name,
            "position": between(last.as_deref(), None),
            "deleted": false,
        }));
        self.write("list", &id, fields)?;
        self.selection.list = Some(id.clone());
        self.selection.item = None;
        Ok(id)
    }

    pub fn rename_list(&mut self, id: Option<&str>, name: &str) -> Result<(), String> {
        let id = self.list_id(id)?;
        let name = name.trim();
        if name.is_empty() {
            return Err("a name can't be empty".into());
        }
        let name = self.unique_name(name, Some(&id));
        self.write("list", &id, object(json!({ "name": name })))
    }

    /// Deletes a list and its items. If it was selected, the next list is selected instead.
    pub fn delete_list(&mut self, id: Option<&str>) -> Result<(), String> {
        let id = self.list_id(id)?;
        let order: Vec<String> = self.lists().into_iter().map(|list| list.id).collect();
        let items: Vec<String> = self.items(&id).all().map(|item| item.id.clone()).collect();
        for item in items {
            self.write("item", &item, object(json!({ "deleted": true })))?;
        }
        self.write("list", &id, object(json!({ "deleted": true })))?;
        if self
            .selection
            .list
            .as_deref()
            .is_none_or(|selected| selected == id)
        {
            let position = order
                .iter()
                .position(|existing| *existing == id)
                .unwrap_or(0);
            let remaining = self.lists();
            self.selection.list = remaining
                .get(position)
                .or(remaining.last())
                .map(|list| list.id.clone());
            self.selection.item = None;
        }
        Ok(())
    }

    /// Adds an item to the end of a list's items to do and selects it. Returns its id.
    pub fn add_item(
        &mut self,
        list: Option<&str>,
        title: &str,
        due: Due,
    ) -> Result<String, String> {
        let list = self.list_id(list)?;
        let title = title.trim();
        if title.is_empty() {
            return Err("an item needs a title".into());
        }
        let last = self
            .items(&list)
            .to_do
            .last()
            .map(|item| item.position.clone());
        let id = Store::new_id();
        let fields = object(json!({
            "list": list,
            "title": title,
            "done": false,
            "done_at": null,
            "due": due.map(|date| date.to_string()),
            "position": between(last.as_deref(), None),
            "deleted": false,
        }));
        self.write("item", &id, fields)?;
        self.selection.list = Some(list);
        self.selection.item = Some(id.clone());
        Ok(id)
    }

    pub fn rename_item(&mut self, id: Option<&str>, title: &str) -> Result<(), String> {
        let item = self.item_id(id)?;
        let title = title.trim();
        if title.is_empty() {
            return Err("an item needs a title".into());
        }
        self.write("item", &item.id, object(json!({ "title": title })))
    }

    pub fn set_due(&mut self, id: Option<&str>, due: Due) -> Result<(), String> {
        let item = self.item_id(id)?;
        let due = due.map(|date| date.to_string());
        self.write("item", &item.id, object(json!({ "due": due })))
    }

    /// Ticks an item off or unticks it: to `done` if given, else the opposite of what it is.
    pub fn tick(&mut self, id: Option<&str>, done: Option<bool>) -> Result<(), String> {
        let item = self.item_id(id)?;
        let done = done.unwrap_or(!item.done);
        if done == item.done {
            return Ok(());
        }
        let done_at = done.then(|| Timestamp::now().as_millisecond());
        self.write(
            "item",
            &item.id,
            object(json!({ "done": done, "done_at": done_at })),
        )
    }

    /// Deletes an item. If it was selected, the next one in the list is selected instead.
    pub fn delete_item(&mut self, id: Option<&str>) -> Result<(), String> {
        let item = self.item_id(id)?;
        let order: Vec<String> = self
            .items(&item.list)
            .all()
            .map(|item| item.id.clone())
            .collect();
        self.write("item", &item.id, object(json!({ "deleted": true })))?;
        if self.selection.item.as_deref() == Some(item.id.as_str()) {
            let position = order
                .iter()
                .position(|existing| *existing == item.id)
                .unwrap_or(0);
            let remaining: Vec<String> = self
                .items(&item.list)
                .all()
                .map(|item| item.id.clone())
                .collect();
            self.selection.item = remaining.get(position).or(remaining.last()).cloned();
        }
        Ok(())
    }

    /// Moves an item among a list's items to do: to `index` (the end if none) of `list` (its
    /// own if none).
    pub fn move_item(
        &mut self,
        id: Option<&str>,
        list: Option<&str>,
        index: Option<usize>,
    ) -> Result<(), String> {
        let item = self.item_id(id)?;
        let list = match list {
            Some(list) => self.list_id(Some(list))?,
            None => item.list.clone(),
        };
        let others: Vec<Item> = self
            .items(&list)
            .to_do
            .into_iter()
            .filter(|other| other.id != item.id)
            .collect();
        let index = index.unwrap_or(others.len()).min(others.len());
        let before = index.checked_sub(1).and_then(|index| others.get(index));
        let after = others.get(index);
        let position = between(
            before.map(|item| item.position.as_str()),
            after.map(|item| item.position.as_str()),
        );
        self.write(
            "item",
            &item.id,
            object(json!({ "list": list, "position": position })),
        )?;
        if self.selection.item.as_deref() == Some(item.id.as_str()) {
            self.selection.list = Some(list);
        }
        Ok(())
    }

    /// Moves the selected item up (negative) or down (positive) among the items to do.
    pub fn move_by(&mut self, offset: isize) -> Result<(), String> {
        let item = self.item_id(None)?;
        if item.done {
            return Err("done items are kept in the order they were ticked off".into());
        }
        let to_do = self.items(&item.list).to_do;
        let index = to_do
            .iter()
            .position(|other| other.id == item.id)
            .unwrap_or(0);
        let Some(target) = index
            .checked_add_signed(offset)
            .filter(|&target| target < to_do.len())
        else {
            return Ok(());
        };
        self.move_item(Some(&item.id), None, Some(target))
    }

    /// Deletes the items a list has ticked off.
    pub fn clear_done(&mut self, list: Option<&str>) -> Result<(), String> {
        let list = self.list_id(list)?;
        let done: Vec<String> = self
            .items(&list)
            .done
            .into_iter()
            .map(|item| item.id)
            .collect();
        if done.is_empty() {
            return Err("nothing in that list is done".into());
        }
        for id in done {
            self.write("item", &id, object(json!({ "deleted": true })))?;
        }
        Ok(())
    }

    /// Makes `requested` unique among the other lists' names the way Finder names files: a
    /// taken name gets the next free number, so "Groceries" becomes "Groceries 2" and "List 2"
    /// becomes "List 3". As with Mac filenames, names that differ only in case collide.
    fn unique_name(&self, requested: &str, renaming: Option<&str>) -> String {
        let names: Vec<String> = self
            .lists()
            .into_iter()
            .filter(|list| Some(list.id.as_str()) != renaming)
            .map(|list| list.name.to_lowercase())
            .collect();
        let taken = |name: &str| names.contains(&name.to_lowercase());
        if !taken(requested) {
            return requested.to_string();
        }
        let (base, number) = requested
            .rsplit_once(' ')
            .and_then(|(base, number)| Some((base, number.parse::<u64>().ok()?)))
            .unwrap_or((requested, 1));
        (number + 1..)
            .map(|number| format!("{base} {number}"))
            .find(|name| !taken(name))
            .expect("some number is free")
    }

    /// Everything an agent needs to know, with due dates read from `today`.
    pub fn report(&self, today: Date) -> Value {
        let lists: Vec<Value> = self
            .lists()
            .into_iter()
            .map(|list| {
                let items = self.items(&list.id);
                let entries: Vec<Value> =
                    items.all().map(|item| item_report(item, today)).collect();
                json!({
                    "id": list.id,
                    "name": list.name,
                    "to_do": items.to_do.len(),
                    "done": items.done.len(),
                    "items": entries,
                })
            })
            .collect();
        let sync = self.sync.as_ref().map(|(folder, problem)| {
            json!({ "folder": folder.display().to_string(), "problem": problem })
        });
        json!({
            "selected_list": self.selected_list_id(),
            "selected_item": self.selected_item_id(),
            "lists": lists,
            "sync": sync,
        })
    }
}

fn item_report(item: &Item, today: Date) -> Value {
    let mut report = json!({ "id": item.id, "title": item.title, "done": item.done });
    if let Some(due) = item.due {
        report["due"] = json!(due.to_string());
        report["due_in_words"] = json!(due::describe(due, today));
        report["overdue"] = json!(!item.done && due < today);
    }
    if let Some(at) = item
        .done_at
        .and_then(|at| Timestamp::from_millisecond(at).ok())
    {
        report["done_at"] = json!(at.to_string());
    }
    report
}

fn list(id: &str, entity: &Entity) -> List {
    List {
        id: id.to_string(),
        name: entity.str("name").unwrap_or_default().to_string(),
        position: entity.str("position").unwrap_or_default().to_string(),
    }
}

fn item(id: &str, entity: &Entity) -> Item {
    Item {
        id: id.to_string(),
        list: entity.str("list").unwrap_or_default().to_string(),
        title: entity.str("title").unwrap_or_default().to_string(),
        done: entity.bool("done"),
        done_at: entity.i64("done_at"),
        due: entity.str("due").and_then(|due| due.parse().ok()),
        position: entity.str("position").unwrap_or_default().to_string(),
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

/// A position that sorts between `before` and `after` (either end open), so an item moves by
/// changing only its own position. Positions are letters read as base-26 fractions; they never
/// end in `a`, so there's always room before one.
pub(crate) fn between(before: Option<&str>, after: Option<&str>) -> String {
    let digits = |text: &str| -> Vec<u8> {
        text.bytes()
            .map(|byte| byte.saturating_sub(b'a').min(25))
            .collect()
    };
    let low = before.map(digits).unwrap_or_default();
    let high = after.map(digits);
    midpoint(&low, high.as_deref())
        .into_iter()
        .map(|digit| char::from(b'a' + digit))
        .collect()
}

/// Digits strictly between `low` and `high` (1, the top, when `None`).
fn midpoint(low: &[u8], high: Option<&[u8]>) -> Vec<u8> {
    if let Some(high) = high {
        let shared = high
            .iter()
            .enumerate()
            .take_while(|(index, digit)| low.get(*index).copied().unwrap_or(0) == **digit)
            .count();
        if shared > 0 {
            let rest = midpoint(low.get(shared..).unwrap_or_default(), Some(&high[shared..]));
            return high[..shared].iter().copied().chain(rest).collect();
        }
    }
    let first_low = u16::from(low.first().copied().unwrap_or(0));
    let first_high = high
        .and_then(|high| high.first().copied())
        .map_or(26, u16::from);
    if first_high > first_low + 1 {
        return vec![((first_low + first_high) / 2) as u8];
    }
    if let Some(high) = high
        && high.len() > 1
    {
        return vec![high[0]];
    }
    let rest = midpoint(low.get(1..).unwrap_or_default(), None);
    std::iter::once(first_low as u8).chain(rest).collect()
}
