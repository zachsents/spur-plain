mod actions;
mod due;
mod main_window;
mod tasks;
mod widget;

use std::{path::PathBuf, rc::Rc, time::Duration};

use futures::StreamExt;

use gpui::{Action, App, AppContext, Entity, KeyBinding, Menu, MenuItem, px, size};
use jiff::Zoned;
use kit::{AppInfo, CloseWindow, EventSource, Occurrence};
use serde::Serialize;
use serde_json::json;
use store::{Applied, Store};

use crate::{
    actions::{
        AddItem, BeginAdd, BeginRename, BeginRenameList, BeginSetDue, ClearDone, CreateList,
        DeleteItem, DeleteList, MoveDown, MoveItem, MoveUp, NewList, RenameItem, RenameList,
        SelectItem, SelectList, SelectNext, SelectNextList, SelectPrevious, SelectPreviousList,
        SetDue, Tick,
    },
    due::Due,
    main_window::MainWindow,
    tasks::{Selection, Tasks},
    widget::ShowList,
};

const APP: AppInfo = AppInfo {
    id: "tasks",
    name: "Tasks",
};

/// The event heard after every change to the data, wherever it was made.
const DATA_CHANGED: &str = "data.changed";

/// How often the other devices' changes are looked for.
const SYNC_EVERY: Duration = Duration::from_secs(4);

fn main() {
    kit::main(APP, |cx| {
        let store = match Store::open(&kit::data_dir(cx).join("tasks.db"), &store::device_id()) {
            Ok(store) => store,
            Err(error) => {
                eprintln!("can't open the lists: {error}");
                cx.quit();
                return;
            }
        };
        let selection: Selection = kit::load(cx, "selection").unwrap_or_default();
        let tasks = cx.new(|_| Tasks::new(store, selection));
        cx.observe(&tasks, |tasks, cx| {
            kit::save(cx, "selection", tasks.read(cx).selection());
            tell_changes(&tasks, cx);
        })
        .detach();
        kit::report_state(cx, {
            let tasks = tasks.clone();
            move |cx| tasks.read(cx).report(Zoned::now().date())
        });
        kit::widget(cx, {
            let tasks = tasks.clone();
            move |arguments: &ShowList, cx| {
                widget::draw(tasks.read(cx), arguments, Zoned::now().date())
            }
        });
        offer_changes(&tasks, cx);

        kit::handle(cx, &tasks, |tasks, action: &NewList| {
            tasks.new_list(action.name.as_deref()).map(|_| ())
        });
        kit::handle(cx, &tasks, |tasks, action: &RenameList| {
            tasks.rename_list(action.id.as_deref(), &action.name)
        });
        kit::handle(cx, &tasks, |tasks, action: &DeleteList| {
            tasks.delete_list(action.id.as_deref())
        });
        kit::handle(cx, &tasks, |tasks, action: &SelectList| {
            tasks.select_list(&action.id)
        });
        kit::handle(cx, &tasks, |tasks, action: &RenameItem| {
            tasks.rename_item(action.id.as_deref(), &action.title)
        });
        kit::handle(cx, &tasks, |tasks, action: &Tick| {
            tasks.tick(action.id.as_deref(), action.done)
        });
        kit::handle(cx, &tasks, |tasks, action: &DeleteItem| {
            tasks.delete_item(action.id.as_deref())
        });
        kit::handle(cx, &tasks, |tasks, action: &MoveItem| {
            tasks.move_item(action.id.as_deref(), action.list.as_deref(), action.index)
        });
        kit::handle(cx, &tasks, |tasks, action: &SelectItem| {
            tasks.select_item(&action.id)
        });
        kit::handle(cx, &tasks, |tasks, action: &ClearDone| {
            tasks.clear_done(action.list.as_deref())
        });
        kit::handle(cx, &tasks, |tasks, _: &MoveUp| tasks.move_by(-1));
        kit::handle(cx, &tasks, |tasks, _: &MoveDown| tasks.move_by(1));
        kit::handle(cx, &tasks, |tasks, _: &SelectPrevious| {
            tasks.select_item_by_offset(-1);
            Ok(())
        });
        kit::handle(cx, &tasks, |tasks, _: &SelectNext| {
            tasks.select_item_by_offset(1);
            Ok(())
        });
        kit::handle(cx, &tasks, |tasks, _: &SelectPreviousList| {
            tasks.select_list_by_offset(-1);
            Ok(())
        });
        kit::handle(cx, &tasks, |tasks, _: &SelectNextList| {
            tasks.select_list_by_offset(1);
            Ok(())
        });
        understand(
            cx,
            &tasks,
            |action: &AddItem| action.due.clone(),
            |tasks, action: &AddItem, due| {
                tasks
                    .add_item(action.list.as_deref(), &action.title, due)
                    .map(|_| ())
            },
        );
        understand(
            cx,
            &tasks,
            |action: &SetDue| Some(action.due.clone()),
            |tasks, action: &SetDue, due| tasks.set_due(action.id.as_deref(), due),
        );

        cx.bind_keys(key_bindings());
        set_menus(cx);
        kit::set_main_window(cx, {
            let tasks = tasks.clone();
            move |cx| {
                let tasks = tasks.clone();
                kit::open_window(cx, APP.name, size(px(680.), px(480.)), |window, cx| {
                    MainWindow::new(tasks, window, cx)
                });
            }
        });
        if let Some(folder) = store::sync_folder(APP.name) {
            keep_in_step(tasks, folder, cx);
        }
    });
}

/// Shares the lists with the person's other devices through `folder`: this device's changes
/// are written there and the others' read in, every few seconds and right after an edit here.
/// The folder is only touched off the main thread, since iCloud Drive may have to download a
/// file first. A problem is kept in the app's state for agents to see, and tried again.
fn keep_in_step(tasks: Entity<Tasks>, folder: PathBuf, cx: &mut App) {
    // An edit here wakes the loop, so it's shared at once. Only a new edit does: a share that
    // failed waits its turn rather than retrying in a tight loop.
    let (kick, mut kicked) = futures::channel::mpsc::unbounded::<()>();
    let seen = std::cell::Cell::new(tasks.read(cx).store().head());
    cx.observe(&tasks, move |tasks, cx| {
        let head = tasks.read(cx).store().head();
        if seen.replace(head) != head {
            kick.unbounded_send(()).ok();
        }
    })
    .detach();
    tasks.update(cx, |tasks, _| tasks.syncing(folder.clone()));
    cx.spawn(async move |cx| {
        let prepared = cx
            .background_executor()
            .spawn({
                let folder = folder.clone();
                async move { store::prepare(&folder) }
            })
            .await;
        if let Err(problem) = prepared {
            tasks.update(cx, |tasks, cx| {
                tasks.sync_problem(Some(problem));
                cx.notify();
            });
            return;
        }
        loop {
            let (device, positions, (unexported, up_to)) = tasks.read_with(cx, |tasks, _| {
                let store = tasks.store();
                (
                    store.device().to_string(),
                    store.read_positions(),
                    store.unexported(),
                )
            });
            let shared = folder.clone();
            let (exported, reads) = cx
                .background_executor()
                .spawn(async move {
                    let exported = store::export(&shared, &device, &unexported);
                    (exported, store::read_peers(&shared, &device, &positions))
                })
                .await;
            tasks.update(cx, |tasks, cx| {
                let problem = exported
                    .and_then(|()| tasks.store_mut().exported(up_to))
                    .and_then(|()| tasks.store_mut().import(reads));
                match problem {
                    Ok(applied) => {
                        tasks.sync_problem(None);
                        if !applied.is_empty() {
                            tasks.merged(applied);
                        }
                    }
                    Err(problem) => tasks.sync_problem(Some(problem)),
                }
                cx.notify();
            });
            let timer = cx.background_executor().timer(SYNC_EVERY);
            futures::future::select(timer, kicked.next()).await;
        }
    })
    .detach();
}

/// Offers the lists' changes to agents as `data.changed`, read from the store, so a host that
/// keeps its own copy, as an assistant on a server would, can follow along and catch up.
fn offer_changes(tasks: &Entity<Tasks>, cx: &mut App) {
    let (epoch, head, read) = (tasks.clone(), tasks.clone(), tasks.clone());
    kit::event_source(
        cx,
        EventSource {
            name: DATA_CHANGED,
            description: "After a change to a list or an item takes effect on this computer, \
                          whether made here, by an agent or on another of the person's devices: \
                          which list or item, and the fields that changed to what. A copy kept \
                          by applying these in order matches the app's. Lists have `name`, \
                          `position` and `deleted`; items have `list`, `title`, `done`, \
                          `done_at`, `due` (YYYY-MM-DD or null), `position` and `deleted`.",
            payload: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["list", "item"] },
                    "id": { "type": "string", "description": "The list's or item's id." },
                    "fields": { "type": "object", "description": "Each field that changed, with its new value." },
                    "device": { "type": "string", "description": "The device the change was made on." },
                },
                "required": ["kind", "id", "fields", "device"],
            }),
            epoch: Box::new(move |cx| epoch.read(cx).store().epoch().to_string()),
            head: Box::new(move |cx| head.read(cx).store().head()),
            read: Box::new(move |after, limit, cx| {
                read.read(cx)
                    .store()
                    .applied_after(after, limit)
                    .iter()
                    .map(occurrence)
                    .collect()
            }),
        },
    );
}

fn occurrence(applied: &Applied) -> Occurrence {
    let change = &applied.change;
    Occurrence {
        seq: applied.seq,
        at: change.stamp.millis,
        id: change.key(),
        data: json!({
            "kind": change.kind,
            "id": change.id,
            "fields": applied.took,
            "device": change.stamp.device,
        }),
    }
}

/// Tells watching agents about changes that took effect since the last time.
fn tell_changes(tasks: &Entity<Tasks>, cx: &mut App) {
    let applied = tasks.update(cx, |tasks, _| tasks.take_unannounced());
    if applied.is_empty() {
        return;
    }
    for applied in applied.iter().filter(|applied| !applied.took.is_empty()) {
        kit::emit(cx, DATA_CHANGED, &occurrence(applied));
    }
    kit::changed(cx);
}

/// Handles an action that carries a due date in words. The words are understood first, which
/// takes a moment when the parser doesn't know them and the decision model is asked, and then
/// `update` gets the date (`None` for no date).
fn understand<A: Action + Clone + Serialize>(
    cx: &mut App,
    tasks: &Entity<Tasks>,
    words: impl Fn(&A) -> Option<String> + 'static,
    update: impl Fn(&mut Tasks, &A, Due) -> Result<(), String> + 'static,
) {
    let tasks = tasks.clone();
    let update = Rc::new(update);
    cx.on_action(move |action: &A, cx| {
        let understood = words(action).map(|words| due::understand(words, cx));
        let (tasks, update, action) = (tasks.clone(), update.clone(), action.clone());
        let announcer = kit::announcer(cx);
        let work = cx.spawn(async move |cx| {
            let due = match understood {
                Some(understood) => understood.await?,
                None => None,
            };
            tasks.update(cx, |tasks, cx| {
                cx.notify();
                update(tasks, &action, due)
            })?;
            cx.update(|cx| announcer.announce(cx, &action));
            Ok(())
        });
        kit::finish_before_replying(cx, work);
    });
}

/// Every shortcut. Single keys only apply outside text fields, so typing a title or a date
/// never triggers them.
fn key_bindings() -> Vec<KeyBinding> {
    let list = Some("Tasks && !TextField");
    vec![
        KeyBinding::new("cmd-n", BeginAdd, Some("Tasks")),
        KeyBinding::new("cmd-shift-n", CreateList, Some("Tasks")),
        KeyBinding::new("enter", BeginRename, list),
        KeyBinding::new("shift-enter", BeginRenameList, list),
        KeyBinding::new("d", BeginSetDue, list),
        KeyBinding::new("space", Tick::default(), list),
        KeyBinding::new("cmd-backspace", DeleteItem::default(), list),
        KeyBinding::new("cmd-alt-backspace", ClearDone::default(), list),
        KeyBinding::new("cmd-shift-backspace", DeleteList::default(), list),
        KeyBinding::new("up", SelectPrevious, list),
        KeyBinding::new("down", SelectNext, list),
        KeyBinding::new("alt-up", SelectPreviousList, list),
        KeyBinding::new("alt-down", SelectNextList, list),
        KeyBinding::new("cmd-alt-up", MoveUp, list),
        KeyBinding::new("cmd-alt-down", MoveDown, list),
    ]
}

fn set_menus(cx: &mut App) {
    kit::set_menus(
        cx,
        APP.name,
        [
            Menu::new("File").items([
                MenuItem::action("New Item", BeginAdd),
                MenuItem::action("New List", CreateList),
                MenuItem::separator(),
                MenuItem::action("Rename", BeginRename),
                MenuItem::action("Due Date", BeginSetDue),
                MenuItem::action("Rename List", BeginRenameList),
                MenuItem::separator(),
                MenuItem::action("Delete", DeleteItem::default()),
                MenuItem::action("Clear Done", ClearDone::default()),
                MenuItem::action("Delete List", DeleteList::default()),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ]),
            Menu::new("Items").items([
                MenuItem::action("Tick", Tick::default()),
                MenuItem::action("Move Up", MoveUp),
                MenuItem::action("Move Down", MoveDown),
                MenuItem::separator(),
                MenuItem::action("Previous", SelectPrevious),
                MenuItem::action("Next", SelectNext),
                MenuItem::action("Previous List", SelectPreviousList),
                MenuItem::action("Next List", SelectNextList),
            ]),
        ],
    );
}
