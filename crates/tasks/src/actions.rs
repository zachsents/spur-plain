//! Every command in the app. Each one has a keyboard shortcut and a menu item, and is an MCP
//! tool for agents: the doc comment is the tool's description and the fields are its input.

use gpui::{Action, actions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Add a list after the others and select it. Without a name it's called "List 1", or the next
/// free number.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct NewList {
    /// The list's name. If another list has it, this one gets the next free number, the way a
    /// file would: "Groceries" becomes "Groceries 2".
    #[serde(default)]
    pub name: Option<String>,
}

/// Rename a list. If another list has the name, this one gets the next free number, the way a
/// file would: "Groceries" becomes "Groceries 2".
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct RenameList {
    /// Which list. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
    /// The new name.
    pub name: String,
}

/// Delete a list and everything in it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct DeleteList {
    /// Which list. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
}

/// Select a list, showing it in the window.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct SelectList {
    /// Which list.
    pub id: String,
}

/// Add an item to the end of a list's items to do, and select it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct AddItem {
    /// Which list. Defaults to the selected one.
    #[serde(default)]
    pub list: Option<String>,
    /// What's to be done.
    pub title: String,
    /// When it's due, in plain words: "today", "tomorrow", "Friday", "next week", "in 3 days",
    /// "Nov 20" or "2026-11-20". Leave it out for no date.
    #[serde(default)]
    pub due: Option<String>,
}

/// Change what an item says.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct RenameItem {
    /// Which item. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
    /// What it should say.
    pub title: String,
}

/// Set when an item is due, or clear it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct SetDue {
    /// Which item. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
    /// When it's due, in plain words: "today", "tomorrow", "Friday", "next week", "in 3 days",
    /// "Nov 20" or "2026-11-20". "none" clears it.
    pub due: String,
}

/// Tick an item off, or untick it if it's ticked. Pass `done` to set it either way, so asking
/// twice changes nothing.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct Tick {
    /// Which item. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
    /// True to tick it off, false to untick it. Defaults to the opposite of what it is.
    #[serde(default)]
    pub done: Option<bool>,
}

/// Delete an item.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct DeleteItem {
    /// Which item. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
}

/// Move an item to a place among a list's items to do, from the same list or another.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct MoveItem {
    /// Which item. Defaults to the selected one.
    #[serde(default)]
    pub id: Option<String>,
    /// Which list it goes to. Defaults to the one it's in.
    #[serde(default)]
    pub list: Option<String>,
    /// Where it goes among the items to do, 0 being the top. Defaults to the end.
    #[serde(default)]
    pub index: Option<usize>,
}

/// Select an item, showing its list in the window.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct SelectItem {
    /// Which item.
    pub id: String,
}

/// Delete the items a list has ticked off.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
#[serde(deny_unknown_fields)]
pub struct ClearDone {
    /// Which list. Defaults to the selected one.
    #[serde(default)]
    pub list: Option<String>,
}

/// Move the selected item up one place among the items to do.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
pub struct MoveUp;

/// Move the selected item down one place among the items to do.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
pub struct MoveDown;

/// Select the item above the selected one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
pub struct SelectPrevious;

/// Select the item below the selected one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
pub struct SelectNext;

/// Select the list above the selected one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
pub struct SelectPreviousList;

/// Select the list below the selected one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, Action)]
#[action(namespace = tasks)]
pub struct SelectNextList;

// Actions that only drive the window, such as starting to edit a name, live in their own
// namespace so they aren't offered to agents. Agents call the commands themselves.
actions!(
    tasks_window,
    [
        /// Start typing a new item at the end of the list.
        BeginAdd,
        /// Edit the selected item's title in place.
        BeginRename,
        /// Edit the selected item's due date in place.
        BeginSetDue,
        /// Edit the selected list's name in place.
        BeginRenameList,
        /// Add a list and start editing its name.
        CreateList,
    ]
);
