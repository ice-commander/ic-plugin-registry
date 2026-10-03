#![allow(clippy::not_unsafe_ptr_arg_deref)]
// The json! in document() nests deeper than the default limit allows.
#![recursion_limit = "512"]

mod registry;

use ic_plugin_api::{
    check_host, HostCheck, IcBytes, IcHost, IcViewVTable, IC_ABI_VERSION, IC_ENABLE_ALWAYS,
    IC_ERR_HOST_TOO_OLD, IC_ERR_HOST_UNKNOWN, IC_ERR_INIT_FAILED, IC_HOST_GTK, IC_OK,
    IC_SIDE_RIGHT,
};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../version.rs"));

ic_plugin_api::declare_about!(
    "ic-registry-dlg",
    "Registry",
    plugins_version!(),
    "Browses and edits the Windows registry"
);

pub const ID: &str = "ic-registry-dlg";
pub const VIEW_ID: &str = "ic.registry";
const ICON: &str = include_str!("../assets/registry.svg");

const TOOLBAR: [(&str, &str); 7] = [
    ("up", include_str!("../assets/arrow-up.svg")),
    ("refresh", include_str!("../assets/refresh.svg")),
    ("new-key", include_str!("../assets/add-folder.svg")),
    ("new-value", include_str!("../assets/add-file.svg")),
    ("edit", include_str!("../assets/edit-property.svg")),
    ("delete", include_str!("../assets/delete-file.svg")),
    ("close", include_str!("../assets/close.svg")),
];

static HOST: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ANSWER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[derive(Default, Clone)]
pub struct Place {
    pub opened: std::collections::BTreeSet<String>,
    /// The key the right pane is listing.
    pub chosen: Option<String>,
    pub chosen_value: Option<String>,
    pub status: String,
}

fn branch(path: &str, name: &str, opened: &std::collections::BTreeSet<String>) -> Value {
    let is_open = opened.contains(path);
    let children: Vec<Value> = if is_open {
        registry::read(path)
            .keys
            .iter()
            .map(|kid| branch(&registry::child_of(path, kid), kid, opened))
            .collect()
    } else {
        Vec::new()
    };
    json!({
        "name": name,
        "path": path,
        // Always true: finding out means reading every child, so an arrow may open onto nothing.
        "expandable": true,
        "expanded": is_open,
        "children": children,
    })
}

fn forest(opened: &std::collections::BTreeSet<String>) -> (Value, Option<String>) {
    let roots = registry::read("");
    let trees = roots
        .keys
        .iter()
        .map(|root| branch(root, root, opened))
        .collect();
    (Value::Array(trees), roots.error)
}

fn place() -> &'static Mutex<Place> {
    static HELD: std::sync::OnceLock<Mutex<Place>> = std::sync::OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Place::default()))
}

fn standing() -> Place {
    place()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn stand_at(next: Place) {
    *place().lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
}

pub fn crumbs_of(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "Registry".to_string()
    } else {
        trimmed.replace('/', " \u{203a} ")
    }
}

fn value_rows(listing: &registry::Listing) -> Value {
    Value::Array(
        listing
            .values
            .iter()
            .map(|value| {
                json!({
                    "name": if value.name.is_empty() { "(Default)" } else { &value.name },
                    "type": value.data.type_name(),
                    "data": value.data.shown(),
                })
            })
            .collect(),
    )
}

fn showing(at: &Place) -> Value {
    let (keys, unreadable) = forest(&at.opened);
    let (values, trouble) = match at.chosen.as_deref() {
        Some(key) => {
            let inside = registry::read(key);
            (value_rows(&inside), inside.error)
        }
        None => (Value::Array(Vec::new()), None),
    };
    json!({
        "set": {
            "data.path": at.chosen.as_deref().map(crumbs_of).unwrap_or_else(|| "Registry".to_string()),
            "data.keys": keys,
            "data.values": values,
            "data.status": if at.status.is_empty() { trouble.or(unreadable).unwrap_or_default() } else { at.status.clone() },
        }
    })
}

/// Carries the current state: the host rebuilds the view from it on every view_invalidate.
pub fn document() -> Value {
    let data = showing(&standing())["set"].clone();
    json!({
        "schema": 1,
        "kind": "ic.registry",
        "data": {
            "path": data["data.path"].clone(),
            "keys": data["data.keys"].clone(),
            "values": data["data.values"].clone(),
            "status": data["data.status"].clone(),
        },
        "fields": [
            { "bind": "chosen_key", "type": "text" },
            { "bind": "chosen_value", "type": "text" }
        ],
        "form": {
            "t": "view",
            "surface": "panel",
            "padding": 0,
            "spacing": 0,
            "children": [
                { "t": "row", "spacing": 6, "padding": 8, "children": [
                    { "t": "button", "id": "up", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/up",
                      "tooltip": { "tr": "registry.up", "en": "Up" },
                      "intent": { "do": "emit", "node": "up" } },
                    { "t": "button", "id": "refresh", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/refresh",
                      "tooltip": { "tr": "registry.refresh", "en": "Refresh" },
                      "intent": { "do": "emit", "node": "refresh" } },
                    { "t": "separator" },
                    { "t": "button", "id": "new_key", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/new-key",
                      "tooltip": { "tr": "registry.new_key", "en": "New key" },
                      "intent": { "do": "emit", "node": "new_key" } },
                    { "t": "button", "id": "new_value", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/new-value",
                      "tooltip": { "tr": "registry.new_value", "en": "New value" },
                      "intent": { "do": "emit", "node": "new_value" } },
                    { "t": "button", "id": "edit", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/edit",
                      "tooltip": { "tr": "registry.edit", "en": "Edit value" },
                      "intent": { "do": "emit", "node": "edit" } },
                    { "t": "button", "id": "delete", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/delete",
                      "tooltip": { "tr": "registry.delete", "en": "Delete" },
                      "intent": { "do": "emit", "node": "delete" } },
                    { "t": "text", "id": "crumbs", "weight": 1, "role": "dim",
                      "text": "{data.path}" },
                    { "t": "button", "id": "leave", "role": "flat", "height": 20,
                      "icon": "asset:ic-registry-dlg/close",
                      "tooltip": { "tr": "registry.close", "en": "Close" },
                      "intent": { "do": "close" } }
                ]},
                { "t": "separator" },
                { "t": "row", "weight": 1, "spacing": 0, "children": [
                    { "t": "column", "weight": 1, "scroll": "vertical", "padding": 8, "children": [
                        { "t": "tree", "id": "keys", "bind": "chosen_key",
                          "rows_key": "keys", "emit": "change", "weight": 1,
                          "columns": [
                              { "key": "name",
                                "title": { "tr": "registry.keys", "en": "Keys" } } ] }
                    ]},
                    { "t": "separator" },
                    { "t": "column", "weight": 2, "scroll": "vertical", "padding": 8, "children": [
                        { "t": "table", "id": "values", "bind": "chosen_value",
                          "rows_key": "values", "emit": "change", "weight": 1,
                          "intent": { "do": "emit", "node": "values" },
                          "columns": [
                              { "key": "name", "width": 180,
                                "title": { "tr": "registry.name", "en": "Name" } },
                              { "key": "type", "width": 120,
                                "title": { "tr": "registry.type", "en": "Type" } },
                              { "key": "data",
                                "title": { "tr": "registry.data", "en": "Data" } } ] }
                    ]}
                ]},
                { "t": "separator" },
                { "t": "text", "id": "status", "role": "dim", "padding": 6,
                  "text": "{data.status}" }
            ]
        }
    })
}

fn row_path(event: &Value, bind: &str) -> Option<String> {
    event["values"][bind]["path"]
        .as_str()
        .map(|path| path.to_string())
}

/// The expanded row arrives in `value`; `values` holds the selection, which is some other row.
fn arrow_path(event: &Value) -> Option<String> {
    event["value"]["path"].as_str().map(|path| path.to_string())
}

fn row_name(event: &Value, bind: &str) -> Option<String> {
    event["values"][bind]["name"]
        .as_str()
        .map(|name| name.to_string())
}

pub fn reply_for(event: &Value, at: &Place) -> (Value, Place) {
    let kind = event["type"].as_str().unwrap_or_default();
    let node = event["node"].as_str().unwrap_or_default();

    match (kind, node) {
        ("opened", _) => {
            let fresh = Place::default();
            (showing(&fresh), fresh)
        }

        // Selecting must not expand: that makes the tree jump about under the pointer.
        ("change", "keys") => match row_path(event, "chosen_key") {
            Some(path) => {
                let next = Place {
                    chosen: Some(path),
                    chosen_value: None,
                    status: String::new(),
                    ..at.clone()
                };
                (showing(&next), next)
            }
            None => (json!({}), at.clone()),
        },

        ("change", "values") => (
            json!({}),
            Place {
                chosen_value: row_name(event, "chosen_value"),
                ..at.clone()
            },
        ),

        ("expand", "keys") | ("collapse", "keys") => match arrow_path(event) {
            Some(path) => {
                let mut opened = at.opened.clone();
                if kind == "expand" {
                    opened.insert(path);
                } else {
                    let under = format!("{path}/");
                    opened.retain(|held| *held != path && !held.starts_with(&under));
                }
                let next = Place {
                    opened,
                    status: String::new(),
                    ..at.clone()
                };
                (showing(&next), next)
            }
            None => (json!({}), at.clone()),
        },

        // A tree has no "up", so this button collapses every branch.
        ("activate", "up") => {
            let next = Place {
                opened: std::collections::BTreeSet::new(),
                status: String::new(),
                ..at.clone()
            };
            (showing(&next), next)
        }

        ("activate", "refresh") => (showing(at), at.clone()),

        ("activate", "new_key") => {
            if at.chosen.is_none() {
                return (
                    json!({ "set": { "data.status": "pick the key to make it in" } }),
                    at.clone(),
                );
            }
            asked(Asking::NewKey);
            put(&json!({
                "heading": { "tr": "registry.new_key", "en": "New key" },
                "detail": at.chosen.as_deref().map(crumbs_of).unwrap_or_else(|| "Registry".to_string()),
                "input": { "variant": "text", "placeholder": { "tr": "registry.key_name", "en": "Name" } },
                "buttons": [
                    { "id": "cancel", "label": { "tr": "common.cancel", "en": "Cancel" } },
                    { "id": "create", "label": { "tr": "registry.create", "en": "Create" }, "role": "primary" }
                ]
            }));
            (json!({}), at.clone())
        }

        ("activate", "new_value") => {
            let under = at.chosen.clone().unwrap_or_default();
            if under.trim_matches('/').is_empty() {
                return (
                    json!({ "set": { "data.status": "pick a key to put the value in" } }),
                    at.clone(),
                );
            }
            asked(Asking::NewValue(under.clone()));
            put(&json!({
                "heading": { "tr": "registry.new_value", "en": "New value" },
                "detail": crumbs_of(&under),
                "input": { "variant": "text", "placeholder": { "tr": "registry.value_name", "en": "Name" } },
                "choice": { "value": "REG_SZ", "options": offered_types() },
                "buttons": [
                    { "id": "cancel", "label": { "tr": "common.cancel", "en": "Cancel" } },
                    { "id": "create", "label": { "tr": "registry.create", "en": "Create" }, "role": "primary" }
                ]
            }));
            (json!({}), at.clone())
        }

        ("activate", "edit") | ("activate", "values") => match being_edited(at) {
            Some((path, name, data)) => {
                asked(Asking::Edit {
                    path,
                    name: name.clone(),
                    was: data.type_name().to_string(),
                });
                put(&json!({
                    "heading": { "tr": "registry.edit", "en": "Edit value" },
                    "detail": format!("{name}  ({})", data.type_name()),
                    "input": {
                        "variant": if matches!(data, registry::Data::Binary(_)) { "hex" } else { "text" },
                        "value": data.editable()
                    },
                    "buttons": [
                        { "id": "cancel", "label": { "tr": "common.cancel", "en": "Cancel" } },
                        { "id": "save", "label": { "tr": "common.save", "en": "Save" }, "role": "primary" }
                    ]
                }));
                (json!({}), at.clone())
            }
            None => (
                json!({ "set": { "data.status": "no value is selected" } }),
                at.clone(),
            ),
        },

        ("activate", "delete") => match chosen_to_delete(at) {
            Some(what) => {
                asked(Asking::Delete(what.clone()));
                put(&json!({
                    "heading": { "tr": "registry.delete_heading", "en": "Delete this?" },
                    "body": { "tr": "registry.delete_body", "en": "It cannot be brought back." },
                    "detail": what.shown(),
                    "buttons": [
                        { "id": "cancel", "label": { "tr": "common.cancel", "en": "Cancel" } },
                        { "id": "delete", "label": { "tr": "registry.delete", "en": "Delete" }, "role": "destructive" }
                    ]
                }));
                (json!({}), at.clone())
            }
            None => (
                json!({ "set": { "data.status": "nothing is selected" } }),
                at.clone(),
            ),
        },

        _ => (json!({}), at.clone()),
    }
}

/// In the order regedit offers them.
pub fn offered_types() -> Value {
    json!([
        { "id": "REG_SZ",        "label": { "tr": "registry.type_sz",        "en": "String" } },
        { "id": "REG_EXPAND_SZ", "label": { "tr": "registry.type_expand_sz", "en": "Expandable string" } },
        { "id": "REG_MULTI_SZ",  "label": { "tr": "registry.type_multi_sz",  "en": "Multi-string" } },
        { "id": "REG_DWORD",     "label": { "tr": "registry.type_dword",     "en": "32-bit number" } },
        { "id": "REG_QWORD",     "label": { "tr": "registry.type_qword",     "en": "64-bit number" } },
        { "id": "REG_BINARY",    "label": { "tr": "registry.type_binary",    "en": "Binary" } }
    ])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asking {
    Nothing,
    NewKey,
    NewValue(String),
    Delete(Target),
    Edit {
        path: String,
        name: String,
        was: String,
    },
}

/// Read back, because the table holds display text rather than what is stored.
pub fn being_edited(at: &Place) -> Option<(String, String, registry::Data)> {
    let name = at.chosen_value.clone()?;
    let path = at.chosen.clone()?;
    let wanted = if name == "(Default)" { "" } else { name.as_str() };
    let found = registry::read(&path)
        .values
        .into_iter()
        .find(|value| value.name == wanted)?;
    Some((path, name, found.data))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Key(String),
    Value { path: String, name: String },
}

impl Target {
    fn shown(&self) -> String {
        match self {
            Target::Key(path) => crumbs_of(path),
            Target::Value { path, name } => format!("{} \u{203a} {name}", crumbs_of(path)),
        }
    }
}

fn pending() -> &'static Mutex<Asking> {
    static HELD: std::sync::OnceLock<Mutex<Asking>> = std::sync::OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Asking::Nothing))
}

fn asked(about: Asking) {
    *pending().lock().unwrap_or_else(|p| p.into_inner()) = about;
}

fn taken() -> Asking {
    let mut held = pending().lock().unwrap_or_else(|p| p.into_inner());
    std::mem::replace(&mut *held, Asking::Nothing)
}

// A selected value wins: deleting the key it sits in would be a surprise.
pub fn chosen_to_delete(at: &Place) -> Option<Target> {
    if let Some(name) = at.chosen_value.clone() {
        return Some(Target::Value {
            path: at.chosen.clone().unwrap_or_default(),
            name,
        });
    }
    at.chosen.clone().map(Target::Key)
}

fn put(spec: &Value) {
    let host = HOST.load(Ordering::Relaxed) as *const IcHost;
    if host.is_null() {
        return;
    }
    let sent = spec.to_string();
    unsafe {
        ((*host).ask)(
            sent.as_ptr(),
            sent.len() as u64,
            answered,
            std::ptr::null_mut(),
        );
    }
}

extern "C" fn answered(answer: *const u8, len: u64, _user_data: *mut c_void) {
    if answer.is_null() || len == 0 {
        return;
    }
    let raw = unsafe { std::slice::from_raw_parts(answer, len as usize) };
    let said: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let button = said["button"].as_str().unwrap_or_default();
    let typed = said["text"].as_str().unwrap_or_default();
    let named = typed.trim();
    let kind = said["choice"].as_str().unwrap_or_default().to_string();
    let about = taken();

    let at = standing();
    let trouble = match (about, button) {
        (Asking::NewKey, "create") if !named.is_empty() => {
            registry::create_key(at.chosen.as_deref().unwrap_or_default(), named).err()
        }
        (Asking::NewValue(path), "create") if !named.is_empty() => {
            let blank = match kind.as_str() {
                "REG_DWORD" | "REG_QWORD" => "0",
                _ => "",
            };
            match registry::Data::from_type_name(&kind, blank) {
                Ok(made) => registry::set_value(&path, named, &made).err(),
                Err(said) => Some(said),
            }
        }
        (Asking::Delete(Target::Key(path)), "delete") => registry::delete(&path, true, None).err(),
        (Asking::Delete(Target::Value { path, name }), "delete") => {
            registry::delete(&path, false, Some(&name)).err()
        }
        (Asking::Edit { path, name, was }, "save") => {
            let under = if name == "(Default)" { "" } else { name.as_str() };
            match registry::Data::from_type_name(&was, typed) {
                Ok(data) => registry::set_value(&path, under, &data).err(),
                Err(said) => Some(said),
            }
        }
        _ => return,
    };

    if let Some(said) = trouble {
        stand_at(Place { status: said, ..at });
    }
    let host = HOST.load(Ordering::Relaxed) as *const IcHost;
    if host.is_null() {
        return;
    }
    if let Ok(id) = CString::new(VIEW_ID) {
        unsafe { ((*host).view_invalidate)(id.as_ptr()) };
    }
}

fn answer_with(source: &str) -> IcBytes {
    ANSWER.with(|slot| {
        *slot.borrow_mut() = source.as_bytes().to_vec();
        let held = slot.borrow();
        IcBytes {
            data: held.as_ptr(),
            len: held.len() as u64,
        }
    })
}

extern "C" fn describe(_ctx: *const u8, _ctx_len: u64, _user_data: *mut c_void) -> IcBytes {
    answer_with(&document().to_string())
}

extern "C" fn on_event(event: *const u8, len: u64, _user_data: *mut c_void) -> IcBytes {
    if event.is_null() || len == 0 {
        return answer_with("{}");
    }
    let raw = unsafe { std::slice::from_raw_parts(event, len as usize) };
    let parsed: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let (reply, next) = reply_for(&parsed, &standing());
    stand_at(next);
    answer_with(&reply.to_string())
}

extern "C" fn on_clicked(_user_data: *mut c_void, _parent_window: *mut c_void) {
    let host = HOST.load(Ordering::Relaxed) as *const IcHost;
    if host.is_null() {
        return;
    }
    let Ok(id) = CString::new(VIEW_ID) else {
        return;
    };
    unsafe {
        ((*host).open_view)(id.as_ptr(), std::ptr::null(), 0);
    }
}

/// A null kind comes from a desktop host older than the `kind` argument.
fn draws_a_panel_toolbar(kind: *const c_char) -> bool {
    kind.is_null() || unsafe { CStr::from_ptr(kind) }.to_str() == Ok(IC_HOST_GTK)
}

#[cfg_attr(feature = "export-abi", no_mangle)]
pub extern "C" fn ic_plugin_init(host: *const IcHost, kind: *const c_char) -> c_int {
    match check_host(host, IC_ABI_VERSION, ic_plugin_api::needs_ask()) {
        HostCheck::Ok => {}
        HostCheck::WrongMagic => return IC_ERR_HOST_UNKNOWN,
        HostCheck::TooOld { .. } | HostCheck::Truncated { .. } => return IC_ERR_HOST_TOO_OLD,
    }
    HOST.store(host as usize, Ordering::Relaxed);

    let (Ok(view_id), Ok(owner), Ok(asset), Ok(svg), Ok(title)) = (
        CString::new(VIEW_ID),
        CString::new(ID),
        CString::new("registry"),
        CString::new(ICON),
        CString::new("Registry"),
    ) else {
        return IC_ERR_INIT_FAILED;
    };

    unsafe {
        ((*host).register_plugin_asset)(
            owner.as_ptr(),
            asset.as_ptr(),
            ICON.as_ptr(),
            ICON.len() as u64,
        );
    }
    for (named, svg) in TOOLBAR {
        let Ok(named) = CString::new(named) else {
            continue;
        };
        unsafe {
            ((*host).register_plugin_asset)(
                owner.as_ptr(),
                named.as_ptr(),
                svg.as_ptr(),
                svg.len() as u64,
            );
        }
    }

    let table = IcViewVTable {
        struct_size: std::mem::size_of::<IcViewVTable>() as u32,
        describe,
        on_event: Some(on_event),
        closed: None,
    };
    let registered = unsafe {
        ((*host).register_view)(
            view_id.as_ptr(),
            title.as_ptr(),
            &table,
            std::ptr::null_mut(),
        )
    };
    if registered != IC_OK {
        return registered;
    }

    if !draws_a_panel_toolbar(kind) {
        return IC_OK;
    }

    unsafe {
        ((*host).add_toolbar_button)(
            view_id.as_ptr(),
            svg.as_ptr(),
            title.as_ptr(),
            IC_SIDE_RIGHT,
            30,
            IC_ENABLE_ALWAYS,
            on_clicked,
            std::ptr::null_mut(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, node: &str, values: Value) -> Value {
        json!({ "v": 1, "type": kind, "node": node, "values": values })
    }

    #[test]
    fn the_document_is_valid_json_with_both_tables_bound() {
        let doc = document();
        let panes = &doc["form"]["children"][2]["children"];
        assert_eq!(panes[0]["children"][0]["id"], json!("keys"));
        assert_eq!(panes[0]["children"][0]["bind"], json!("chosen_key"));
        assert_eq!(panes[2]["children"][0]["id"], json!("values"));
        assert_eq!(panes[2]["children"][0]["bind"], json!("chosen_value"));
    }

    #[test]
    fn every_bound_table_names_a_declared_field() {
        let doc = document();
        let declared: Vec<&str> = doc["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .filter_map(|field| field["bind"].as_str())
            .collect();
        for bind in ["chosen_key", "chosen_value"] {
            assert!(declared.contains(&bind), "{bind} is not declared");
        }
    }

    #[test]
    fn opening_the_window_stands_at_the_list_of_roots() {
        let (reply, next) = reply_for(&event("opened", "", json!({})), &Place::default());
        assert!(next.opened.is_empty());
        assert!(next.chosen.is_none());
        let keys = reply["set"]["data.keys"].as_array().expect("keys");
        let names: Vec<&str> = keys.iter().filter_map(|row| row["name"].as_str()).collect();
        let status = reply["set"]["data.status"].as_str().unwrap_or_default();
        #[cfg(target_os = "windows")]
        {
            assert_eq!(names, registry::ROOTS.to_vec());
            assert!(status.is_empty(), "{status}");
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(names.is_empty());
            assert!(!status.is_empty(), "nothing says why the tree is empty");
        }
    }

    fn branch_event(kind: &str, path: &str) -> Value {
        if kind == "expand" || kind == "collapse" {
            return json!({
                "v": 1,
                "type": kind,
                "node": "keys",
                "values": { "chosen_key": { "name": "other", "path": "HKCR" } },
                "value": { "name": "whatever", "path": path }
            });
        }
        event(
            kind,
            "keys",
            json!({ "chosen_key": { "name": "whatever", "path": path } }),
        )
    }

    #[test]
    fn the_arrow_opens_the_branch_it_sits_on_not_the_selected_one() {
        let at = Place {
            chosen: Some("HKCR".to_string()),
            ..Place::default()
        };
        let (_, after) = reply_for(&branch_event("expand", "HKLM"), &at);
        assert!(after.opened.contains("HKLM"), "{:?}", after.opened);
        assert!(!after.opened.contains("HKCR"), "{:?}", after.opened);
    }

    #[test]
    fn an_arrow_event_carrying_no_row_changes_nothing() {
        let (reply, same) = reply_for(&event("expand", "keys", json!({})), &Place::default());
        assert_eq!(reply, json!({}));
        assert!(same.opened.is_empty());
    }

    #[test]
    fn picking_a_branch_lists_it_without_opening_it() {
        let (_, after) = reply_for(&branch_event("change", "HKLM/SOFTWARE"), &Place::default());
        assert_eq!(after.chosen.as_deref(), Some("HKLM/SOFTWARE"));
        assert!(after.opened.is_empty(), "picking must not open anything");
    }

    #[test]
    fn the_arrow_opens_a_branch_and_closing_it_closes_what_was_inside() {
        let (_, opened) = reply_for(&branch_event("expand", "HKLM"), &Place::default());
        assert!(opened.opened.contains("HKLM"));

        let (_, deeper) = reply_for(&branch_event("expand", "HKLM/SOFTWARE"), &opened);
        assert!(deeper.opened.contains("HKLM/SOFTWARE"));

        let (_, shut) = reply_for(&branch_event("collapse", "HKLM"), &deeper);
        assert!(shut.opened.is_empty(), "{:?}", shut.opened);
    }

    #[test]
    fn closing_a_branch_leaves_its_neighbours_open() {
        let (_, opened) = reply_for(&branch_event("expand", "HKLM"), &Place::default());
        let (_, both) = reply_for(&branch_event("expand", "HKCU"), &opened);
        let (_, shut) = reply_for(&branch_event("collapse", "HKLM"), &both);
        assert!(shut.opened.contains("HKCU"));
        assert!(!shut.opened.contains("HKLM"));
    }

    #[test]
    fn a_branch_whose_name_merely_starts_the_same_is_left_open() {
        let mut opened = std::collections::BTreeSet::new();
        opened.insert("HKLM/Soft".to_string());
        opened.insert("HKLM/Software".to_string());
        let (_, shut) = reply_for(
            &branch_event("collapse", "HKLM/Soft"),
            &Place {
                opened,
                ..Place::default()
            },
        );
        assert!(shut.opened.contains("HKLM/Software"), "{:?}", shut.opened);
    }

    #[test]
    fn folding_everything_away_keeps_what_is_listed_on_the_right() {
        let (_, opened) = reply_for(&branch_event("expand", "HKLM"), &Place::default());
        let (_, picked) = reply_for(&branch_event("change", "HKLM"), &opened);
        let (_, folded) = reply_for(&event("activate", "up", json!({})), &picked);
        assert!(folded.opened.is_empty());
        assert_eq!(folded.chosen.as_deref(), Some("HKLM"));
    }

    #[test]
    fn an_event_about_nothing_changes_nothing() {
        let at = Place {
            chosen: Some("HKCU".to_string()),
            ..Place::default()
        };
        let (reply, same) = reply_for(&event("change", "keys", json!({})), &at);
        assert_eq!(reply, json!({}));
        assert_eq!(same.chosen.as_deref(), Some("HKCU"));
    }

    #[test]
    fn a_selected_value_is_deleted_before_the_key_holding_it() {
        let showing_a_key = Place {
            chosen: Some("HKLM/SOFTWARE".to_string()),
            ..Place::default()
        };
        assert_eq!(
            chosen_to_delete(&showing_a_key),
            Some(Target::Key("HKLM/SOFTWARE".to_string()))
        );

        let value_picked = Place {
            chosen_value: Some("InstallRoot".to_string()),
            ..showing_a_key.clone()
        };
        assert_eq!(
            chosen_to_delete(&value_picked),
            Some(Target::Value {
                path: "HKLM/SOFTWARE".to_string(),
                name: "InstallRoot".to_string()
            })
        );

        assert_eq!(chosen_to_delete(&Place::default()), None);
    }

    #[test]
    fn picking_a_value_is_remembered_and_changing_key_forgets_it() {
        let at = Place {
            chosen: Some("HKLM/SOFTWARE".to_string()),
            chosen_value: Some("Old".to_string()),
            ..Place::default()
        };
        let picked = event(
            "change",
            "values",
            json!({ "chosen_value": { "name": "Version" } }),
        );
        let (_, after) = reply_for(&picked, &at);
        assert_eq!(after.chosen_value.as_deref(), Some("Version"));

        let moved = branch_event("change", "HKLM/Other");
        let (_, elsewhere) = reply_for(&moved, &after);
        assert!(elsewhere.chosen_value.is_none());
    }

    #[test]
    fn every_offered_type_is_one_the_backend_can_actually_write() {
        let offered = offered_types();
        let listed = offered.as_array().expect("a list");
        assert_eq!(listed.len(), 6);
        for one in listed {
            let id = one["id"].as_str().expect("an id");
            let blank = match id {
                "REG_DWORD" | "REG_QWORD" => "0",
                _ => "",
            };
            let made = registry::Data::from_type_name(id, blank)
                .unwrap_or_else(|said| panic!("{id} is offered but refused: {said}"));
            assert_eq!(made.type_name(), id);
        }
    }

    #[test]
    fn a_value_cannot_be_made_at_the_list_of_roots() {
        let (reply, _) = reply_for(&event("activate", "new_value", json!({})), &Place::default());
        assert!(
            reply["set"]["data.status"]
                .as_str()
                .unwrap_or_default()
                .contains("pick a key"),
            "{reply}"
        );
    }

    #[test]
    fn the_crumbs_read_as_a_path_rather_than_a_slash_salad() {
        assert_eq!(crumbs_of(""), "Registry");
        assert_eq!(crumbs_of("/"), "Registry");
        assert_eq!(crumbs_of("HKLM/SOFTWARE"), "HKLM \u{203a} SOFTWARE");
    }
}
