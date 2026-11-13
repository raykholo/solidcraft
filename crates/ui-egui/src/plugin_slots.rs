//! Manifest placements drawn by the host; entry requests share the host plugin dialog.
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::{SolveApp, plugins, ui_kit};
#[cfg(not(target_arch = "wasm32"))]
use solidcraft_engine::plugin::UiSlot;

#[derive(Clone)]
pub(crate) struct Item {
    pub command: String,
    pub plugin: String,
    pub entry: String,
    pub label: String,
    pub slot: Slot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Menu(&'static str),
    Ribbon(&'static str),
}
#[derive(Default)]
pub(crate) struct State {
    items: Vec<Item>,
    scanned: Option<f64>,
    discovery: Option<Receiver<Vec<Item>>>,
    entry_request: Option<String>,
}
impl State {
    pub(crate) fn discovery_pending(&self) -> bool {
        self.discovery.is_some()
    }
}
pub(crate) fn ribbon_name(name: &str) -> &'static str {
    crate::workspace::choosable_tabs().chain(["SKETCH"]).find(|tab| tab.eq_ignore_ascii_case(name.trim())).unwrap_or("PLUGINS")
}
pub(crate) fn menu_name(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "file" => "File",
        "help" => "Help",
        _ => "Plugins",
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn declared(plugin: &plugins::Plugin) -> Vec<Item> {
    let mut items = Vec::new();
    for entry in &plugin.entries {
        for slot in &entry.slots {
            let slot = match slot {
                UiSlot::Menu { name } => Slot::Menu(menu_name(name)),
                UiSlot::Ribbon { name } => Slot::Ribbon(ribbon_name(name)),
            };
            let command = format!("{}.{}", plugin.id, entry.name);
            if solidcraft_engine::find_command(&command).is_some() {
                continue;
            }
            if items.iter().any(|item: &Item| item.command == command && item.slot == slot) {
                continue;
            }
            items.push(Item {
                command,
                plugin: plugin.name.clone(),
                entry: entry.name.clone(),
                label: format!("{} · {}", plugin.name, entry.name),
                slot,
            });
        }
    }
    items
}
pub(crate) fn invalidate(app: &mut SolveApp) {
    app.plugin_slots.discovery = None;
    app.plugin_slots.items.clear();
    app.plugin_slots.scanned = None;
}
pub(crate) fn refresh(app: &mut SolveApp) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let now = crate::now_ms();
    if let Some(rx) = &app.plugin_slots.discovery {
        match rx.try_recv() {
            Ok(items) => {
                app.plugin_slots.items = items;
                app.plugin_slots.discovery = None;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                app.plugin_slots.discovery = None;
            }
        }
    }
    if app.plugin_slots.scanned.is_some_and(|at| now - at < 1000.0) {
        return;
    }
    let Some(root) = app.session.plugin_state.install_root.clone().or_else(plugins::plugins_dir) else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new()
        .name("plugin-discovery".into())
        .spawn(move || {
            let _ = tx.send(plugins::scan(&root).0.iter().flat_map(declared).take(8192).collect());
        })
        .is_err()
    {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (root, tx);
    app.plugin_slots.discovery = Some(rx);
    app.plugin_slots.scanned = Some(now);
}
pub(crate) fn items(app: &SolveApp, slot: Slot) -> Vec<Item> {
    app.plugin_slots
        .items
        .iter()
        .filter(|item| item.slot == slot && item.command.split_once('.').is_some_and(|(id, _)| plugins::enabled(app, id)))
        .cloned()
        .collect()
}
pub(crate) fn has_fallback_tab(app: &SolveApp) -> bool {
    !items(app, Slot::Ribbon("PLUGINS")).is_empty()
}

/// Queue the namespaced command for the host's shared input-schema/approval dialog.
/// The generic Plugins menu consumes the same request; slots never execute plugin code.
pub(crate) fn begin(app: &mut SolveApp, command: &str) {
    let Some((id, entry)) = command.split_once('.') else { return };
    if plugins::valid_id(id) && plugins::valid_id(entry) && plugins::enabled(app, id) && solidcraft_engine::find_command(command).is_none() {
        app.plugin_slots.entry_request = Some(command.to_owned());
    }
}

impl SolveApp {
    /// Consume a declared menu/ribbon click in the shared plugin entry dialog.
    /// Open the entry's input_schema form, then use the normal namespaced plugin.run approval path.
    pub fn take_plugin_entry_request(&mut self) -> Option<String> {
        self.plugin_slots.entry_request.take().filter(|command| command.split_once('.').is_some_and(|(id, _)| plugins::enabled(self, id)))
    }

    /// Rows for unknown declared menu names, drawn by the host's generic Plugins menu.
    /// Each row already has the plugin's name and the existing plugin badge.
    pub fn plugin_fallback_menu_items(&self) -> Vec<ui_kit::Item> {
        items(self, Slot::Menu("Plugins")).into_iter().map(|item| ui_kit::Item::action(&item.command, &item.label, "prefs_plugins")).collect()
    }
}

pub(crate) fn menu_rows(app: &mut SolveApp, ui: &mut egui::Ui, name: &'static str) -> bool {
    let entries = items(app, Slot::Menu(name));
    if entries.is_empty() {
        return false;
    }
    ui.separator();
    for item in entries {
        let row = ui_kit::Item::action(&item.command, &item.label, "prefs_plugins");
        if ui_kit::menu::list_row(ui, &row).is_some() {
            begin(app, &item.command);
            return true;
        }
    }
    false
}

pub(crate) struct RibbonGroup {
    key: String,
    name: String,
    entries: Vec<Item>,
    popup: egui::Id,
    open: bool,
}
pub(crate) fn ribbon_groups(app: &SolveApp, ui: &egui::Ui, tab: &str) -> Vec<RibbonGroup> {
    let mut groups: Vec<RibbonGroup> = Vec::new();
    for item in items(app, Slot::Ribbon(ribbon_name(tab))) {
        let Some((id, _)) = item.command.split_once('.') else { continue };
        let key = format!("{tab}:plugin:{id}");
        if let Some(group) = groups.iter_mut().find(|g| g.key == key) {
            group.entries.push(item);
        } else {
            let popup = ui.scope_id().with(("plugin_ribbon", &key));
            let open = ui.ctx().data(|d| d.get_temp::<egui::Id>(egui::Id::unique("sc_panel_open"))) == Some(popup);
            groups.push(RibbonGroup { key, name: item.plugin.clone(), entries: vec![item], popup, open });
        }
    }
    groups
}
impl RibbonGroup {
    pub(crate) fn group(&self, ui: &egui::Ui) -> crate::ribbon::Group<'_> {
        use crate::ribbon;
        ribbon::Group {
            key: &self.key,
            name: &self.name,
            label: &self.name,
            tip: &self.name,
            buttons: self
                .entries
                .iter()
                .map(|entry| ribbon::Button {
                    id: &entry.command,
                    run: &entry.command,
                    label: &entry.entry,
                    name: &entry.label,
                    icon: "prefs_plugins",
                    size: ribbon::ButtonSize { width: ribbon::button_width(ui.painter(), &entry.entry), min_width: ribbon::BUTTON_MIN_WIDTH },
                    enabled: true,
                    pressed: false,
                    exit: false,
                    split: None,
                    app_paints: false,
                    covers: &[],
                })
                .collect(),
            cells: vec![],
            commands: self.entries.iter().map(|e| e.command.as_str()).collect(),
            icon: Some("prefs_plugins"),
            enabled: true,
            open: self.open,
            running: None,
            more: self
                .entries
                .iter()
                .map(|e| ribbon::MoreRow {
                    id: &e.command,
                    label: &e.label,
                    icon: "prefs_plugins",
                    shortcut: "",
                    enabled: true,
                    checked: None,
                    hint: None,
                })
                .collect(),
        }
    }
    pub(crate) fn toggle(&self, ctx: &egui::Context, toggle: bool) {
        ctx.data_mut(|d| {
            let key = egui::Id::unique("sc_panel_open");
            if toggle && d.get_temp::<egui::Id>(key) == Some(self.popup) {
                d.remove::<egui::Id>(key);
            } else {
                d.insert_temp(key, self.popup);
            }
        });
    }
    pub(crate) fn popup(&self, app: &mut SolveApp, ui: &egui::Ui, drawn: &crate::ribbon::GroupOut) {
        let ctx = ui.ctx();
        if ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::unique("sc_panel_open"))) != Some(self.popup) {
            return;
        }
        let mut chosen = None;
        let out = ui_kit::Container::new(ctx, ui_kit::Surface::Popup)
            .area(self.popup)
            .fixed_pos(drawn.label.left_bottom())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui_kit::Menu::show_frame(ui, ui_kit::Menu::frame(), |ui| {
                    ui_kit::Menu::heading(ui, &self.name);
                    for item in &self.entries {
                        if ui_kit::menu::list_row(ui, &ui_kit::Item::action(&item.command, &item.label, "prefs_plugins")).is_some() {
                            chosen = Some(item.command.clone());
                        }
                    }
                });
            });
        if let Some(command) = chosen {
            begin(app, &command);
            ctx.data_mut(|d| d.remove::<egui::Id>(egui::Id::unique("sc_panel_open")));
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.any_pressed())
            && !out.response.contains_pointer()
            && !drawn.label_response.contains_pointer()
        {
            ctx.data_mut(|d| d.remove::<egui::Id>(egui::Id::unique("sc_panel_open")));
        }
    }
}

/// The finish group remains last; the app still addresses its original built-in group indices.
pub(crate) fn restore_group_indices(out: &mut crate::ribbon::Out, builtin: usize, plugins: usize, exit: bool) {
    if !exit || builtin == 0 || plugins == 0 {
        return;
    }
    let original = |group: usize| {
        if group == builtin + plugins - 1 {
            builtin - 1
        } else if group >= builtin - 1 {
            group + 1
        } else {
            group
        }
    };
    for button in &mut out.buttons {
        button.group = original(button.group);
    }
    for group in &mut out.groups {
        group.group = original(group.group);
    }
    for action in &mut out.actions {
        match action {
            crate::ribbon::Action::Section { group } | crate::ribbon::Action::Open { group } | crate::ribbon::Action::Drop { group, .. } => {
                *group = original(*group)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub(crate) fn wait_for_discovery(app: &mut SolveApp) {
    if let Some(rx) = app.plugin_slots.discovery.take() {
        app.plugin_slots.items = rx.recv().unwrap();
    }
}
