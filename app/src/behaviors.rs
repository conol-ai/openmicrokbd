//! User-facing key behaviors and their execution mappings.
//!
//! The editor talks in terms of application shortcuts, macOS controls,
//! keystrokes, and apps. This module translates those choices into the
//! existing device slot plus optional host action without leaking that split
//! into the UI.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Action, ControlBehavior, InputConfig, MacOsControl, Profile, Slot, SlotKind};
use crate::keycodes::{keyboard_name, mods_label};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutPreset {
    pub id: &'static str,
    /// Command label per language, in `[en, zh-Hans, zh-Hant, ja]` order —
    /// the app's own localized menu wording where it ships that localization.
    pub labels: [&'static str; 4],
    pub mods: u8,
    pub key: u16,
}

impl ShortcutPreset {
    pub fn label(&self) -> &'static str {
        self.labels[crate::i18n::lang_index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutApplication {
    pub id: &'static str,
    pub label: &'static str,
    /// Picker artwork: `simple:<slug>` brand icon or a bare Lucide name.
    pub icon: &'static str,
    pub shortcuts: &'static [ShortcutPreset],
}

// --- generated shortcut catalog begin ---
const FINDER_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_window", ["New Finder window", "New Finder window", "New Finder window", "New Finder window"], 0x08, 0x11),
    shortcut("new_folder", ["New folder", "New folder", "New folder", "New folder"], 0x0a, 0x11),
    shortcut("go_to_folder", ["Go to folder", "Go to folder", "Go to folder", "Go to folder"], 0x0a, 0x0a),
    shortcut("get_info", ["Get info", "Get info", "Get info", "Get info"], 0x08, 0x0c),
    shortcut("quick_look", ["Quick Look", "Quick Look", "Quick Look", "Quick Look"], 0x00, 0x2c),
    shortcut("move_to_trash", ["Move to Trash", "Move to Trash", "Move to Trash", "Move to Trash"], 0x08, 0x2a),
    shortcut("new_tab", ["New tab", "New tab", "New tab", "New tab"], 0x08, 0x17),
    shortcut("downloads_folder", ["Open Downloads folder", "Open Downloads folder", "Open Downloads folder", "Open Downloads folder"], 0x0c, 0x0f),
    shortcut("home_folder", ["Open Home folder", "Open Home folder", "Open Home folder", "Open Home folder"], 0x0a, 0x0b),
    shortcut("duplicate", ["Duplicate", "Duplicate", "Duplicate", "Duplicate"], 0x08, 0x07),
];

const SAFARI_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_tab", ["New tab", "New tab", "New tab", "New tab"], 0x08, 0x17),
    shortcut("close_tab", ["Close tab", "Close tab", "Close tab", "Close tab"], 0x08, 0x1a),
    shortcut("reopen_tab", ["Reopen last closed tab", "Reopen last closed tab", "Reopen last closed tab", "Reopen last closed tab"], 0x0a, 0x17),
    shortcut("address", ["Focus address bar", "Focus address bar", "Focus address bar", "Focus address bar"], 0x08, 0x0f),
    shortcut("downloads", ["Show downloads", "Show downloads", "Show downloads", "Show downloads"], 0x0c, 0x0f),
    shortcut("private_window", ["New Private Window", "New Private Window", "New Private Window", "New Private Window"], 0x0a, 0x11),
    shortcut("reload", ["Reload Page", "Reload Page", "Reload Page", "Reload Page"], 0x08, 0x15),
    shortcut("reader", ["Show Reader", "Show Reader", "Show Reader", "Show Reader"], 0x0a, 0x15),
    shortcut("tab_overview", ["Show Tab Overview", "Show Tab Overview", "Show Tab Overview", "Show Tab Overview"], 0x0a, 0x31),
    shortcut("add_reading_list", ["Add to Reading List", "Add to Reading List", "Add to Reading List", "Add to Reading List"], 0x0a, 0x07),
];

const CHROME_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_tab", ["New Tab", "New Tab", "New Tab", "New Tab"], 0x08, 0x17),
    shortcut("close_tab", ["Close Tab", "Close Tab", "Close Tab", "Close Tab"], 0x08, 0x1a),
    shortcut("reopen_tab", ["Reopen Closed Tab", "Reopen Closed Tab", "Reopen Closed Tab", "Reopen Closed Tab"], 0x0a, 0x17),
    shortcut("address", ["Open Location", "Open Location", "Open Location", "Open Location"], 0x08, 0x0f),
    shortcut("incognito", ["New incognito window", "New incognito window", "New incognito window", "New incognito window"], 0x0a, 0x11),
    shortcut("new_window", ["New Window", "New Window", "New Window", "New Window"], 0x08, 0x11),
    shortcut("reload", ["Reload This Page", "Reload This Page", "Reload This Page", "Reload This Page"], 0x08, 0x15),
    shortcut("bookmarks_bar", ["Always Show Bookmarks Bar", "Always Show Bookmarks Bar", "Always Show Bookmarks Bar", "Always Show Bookmarks Bar"], 0x0a, 0x05),
    shortcut("downloads", ["Downloads", "Downloads", "Downloads", "Downloads"], 0x0a, 0x0d),
    shortcut("dev_tools", ["Developer Tools", "Developer Tools", "Developer Tools", "Developer Tools"], 0x0c, 0x0c),
];

const VSCODE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("command_palette", ["Command Palette", "Command Palette", "Command Palette", "Command Palette"], 0x0a, 0x13),
    shortcut("quick_open", ["Quick Open", "Quick Open", "Quick Open", "Quick Open"], 0x08, 0x13),
    shortcut("toggle_terminal", ["Toggle terminal", "Toggle terminal", "Toggle terminal", "Toggle terminal"], 0x01, 0x35),
    shortcut("toggle_sidebar", ["Toggle Primary Side Bar", "Toggle Primary Side Bar", "Toggle Primary Side Bar", "Toggle Primary Side Bar"], 0x08, 0x05),
    shortcut("toggle_panel", ["Toggle Panel", "Toggle Panel", "Toggle Panel", "Toggle Panel"], 0x08, 0x0d),
    shortcut("find_in_files", ["Find in files", "Find in files", "Find in files", "Find in files"], 0x0a, 0x09),
    shortcut("start_debugging", ["Start Debugging", "Start Debugging", "Start Debugging", "Start Debugging"], 0x00, 0x3e),
    shortcut("split_editor", ["Split Editor", "Split Editor", "Split Editor", "Split Editor"], 0x08, 0x31),
    shortcut("extensions", ["Extensions", "Extensions", "Extensions", "Extensions"], 0x0a, 0x1b),
    shortcut("new_window", ["New window", "New window", "New window", "New window"], 0x0a, 0x11),
];

const XCODE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("build", ["Build", "Build", "Build", "Build"], 0x08, 0x05),
    shortcut("run", ["Run", "Run", "Run", "Run"], 0x08, 0x15),
    shortcut("stop", ["Stop", "Stop", "Stop", "Stop"], 0x08, 0x37),
    shortcut("test", ["Test", "Test", "Test", "Test"], 0x08, 0x18),
    shortcut("clean_build_folder", ["Clean Build Folder", "Clean Build Folder", "Clean Build Folder", "Clean Build Folder"], 0x0a, 0x0e),
    shortcut("open_quickly", ["Open Quickly", "Open Quickly", "Open Quickly", "Open Quickly"], 0x0a, 0x12),
    shortcut("debug_area", ["Show Debug Area", "Show Debug Area", "Show Debug Area", "Show Debug Area"], 0x0a, 0x1c),
    shortcut("navigator", ["Show/Hide Navigator", "Show/Hide Navigator", "Show/Hide Navigator", "Show/Hide Navigator"], 0x08, 0x27),
    shortcut("inspectors", ["Show/Hide Inspectors", "Show/Hide Inspectors", "Show/Hide Inspectors", "Show/Hide Inspectors"], 0x0c, 0x27),
    shortcut("library", ["Show Library", "Show Library", "Show Library", "Show Library"], 0x0a, 0x0f),
];

const TERMINAL_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_window", ["New window", "New window", "New window", "New window"], 0x08, 0x11),
    shortcut("new_tab", ["New tab", "New tab", "New tab", "New tab"], 0x08, 0x17),
    shortcut("clear", ["Clear to Start", "Clear to Start", "Clear to Start", "Clear to Start"], 0x08, 0x0e),
    shortcut("close", ["Close Tab", "Close Tab", "Close Tab", "Close Tab"], 0x08, 0x1a),
    shortcut("find", ["Find", "Find", "Find", "Find"], 0x08, 0x09),
    shortcut("split_pane", ["Split Pane", "Split Pane", "Split Pane", "Split Pane"], 0x08, 0x07),
    shortcut("next_tab", ["Next Tab", "Next Tab", "Next Tab", "Next Tab"], 0x01, 0x2b),
    shortcut("show_inspector", ["Show Inspector", "Show Inspector", "Show Inspector", "Show Inspector"], 0x08, 0x0c),
    shortcut("previous_mark", ["Jump to Previous Mark", "Jump to Previous Mark", "Jump to Previous Mark", "Jump to Previous Mark"], 0x08, 0x52),
    shortcut("next_mark", ["Jump to Next Mark", "Jump to Next Mark", "Jump to Next Mark", "Jump to Next Mark"], 0x08, 0x51),
];

const SLACK_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_switcher", ["Quick switcher", "Quick switcher", "Quick switcher", "Quick switcher"], 0x08, 0x0e),
    shortcut("unreads", ["All unreads", "All unreads", "All unreads", "All unreads"], 0x0a, 0x04),
    shortcut("dms", ["Direct messages", "Direct messages", "Direct messages", "Direct messages"], 0x0a, 0x0e),
    shortcut("activity", ["Activity", "Activity", "Activity", "Activity"], 0x0a, 0x10),
    shortcut("threads", ["Threads", "Threads", "Threads", "Threads"], 0x0a, 0x17),
    shortcut("history_back", ["Previous page", "Previous page", "Previous page", "Previous page"], 0x08, 0x2f),
    shortcut("history_forward", ["Next page", "Next page", "Next page", "Next page"], 0x08, 0x30),
    shortcut("preferences", ["Preferences", "Preferences", "Preferences", "Preferences"], 0x08, 0x36),
];

const FIGMA_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_actions", ["Quick actions", "Quick actions", "Quick actions", "Quick actions"], 0x08, 0x38),
    shortcut("move", ["Move tool", "Move tool", "Move tool", "Move tool"], 0x00, 0x19),
    shortcut("frame", ["Frame tool", "Frame tool", "Frame tool", "Frame tool"], 0x00, 0x09),
    shortcut("pen", ["Pen tool", "Pen tool", "Pen tool", "Pen tool"], 0x00, 0x13),
    shortcut("text", ["Text tool", "Text tool", "Text tool", "Text tool"], 0x00, 0x17),
    shortcut("rectangle", ["Rectangle tool", "Rectangle tool", "Rectangle tool", "Rectangle tool"], 0x00, 0x15),
    shortcut("components", ["Create component", "Create component", "Create component", "Create component"], 0x0c, 0x0e),
    shortcut("auto_layout", ["Add auto layout", "Add auto layout", "Add auto layout", "Add auto layout"], 0x02, 0x04),
    shortcut("toggle_ui", ["Show/hide UI", "Show/hide UI", "Show/hide UI", "Show/hide UI"], 0x08, 0x31),
    shortcut("comment", ["Add comment", "Add comment", "Add comment", "Add comment"], 0x00, 0x06),
];

const ZOOM_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("mute", ["Mute or unmute", "Mute or unmute", "Mute or unmute", "Mute or unmute"], 0x0a, 0x04),
    shortcut("video", ["Start or stop video", "Start or stop video", "Start or stop video", "Start or stop video"], 0x0a, 0x19),
    shortcut("share", ["Start or stop screen share", "Start or stop screen share", "Start or stop screen share", "Start or stop screen share"], 0x0a, 0x16),
    shortcut("raise_hand", ["Raise or lower hand", "Raise or lower hand", "Raise or lower hand", "Raise or lower hand"], 0x04, 0x1c),
    shortcut("record", ["Start/stop local recording", "Start/stop local recording", "Start/stop local recording", "Start/stop local recording"], 0x0a, 0x15),
    shortcut("chat", ["Show meeting chat", "Show meeting chat", "Show meeting chat", "Show meeting chat"], 0x0a, 0x0b),
    shortcut("participants", ["Show participants", "Show participants", "Show participants", "Show participants"], 0x08, 0x18),
    shortcut("gallery_view", ["Speaker or gallery view", "Speaker or gallery view", "Speaker or gallery view", "Speaker or gallery view"], 0x0a, 0x1a),
    shortcut("fullscreen", ["Enter or exit full screen", "Enter or exit full screen", "Enter or exit full screen", "Enter or exit full screen"], 0x0a, 0x09),
    shortcut("invite", ["Invite participants", "Invite participants", "Invite participants", "Invite participants"], 0x08, 0x0c),
];

const DAVINCIRESOLVE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("add_serial_node", ["Add Serial Node", "Add Serial Node", "Add Serial Node", "Add Serial Node"], 0x04, 0x16),
    shortcut("grab_still", ["Grab Still", "Grab Still", "Grab Still", "Grab Still"], 0x0c, 0x0a),
    shortcut("bypass_color_grades", ["Bypass Color Grades", "Bypass Color Grades", "Bypass Color Grades", "Bypass Color Grades"], 0x02, 0x07),
    shortcut("highlight", ["Highlight", "Highlight", "Highlight", "Highlight"], 0x02, 0x0b),
    shortcut("add_layer_node", ["Add Layer Node", "Add Layer Node", "Add Layer Node", "Add Layer Node"], 0x04, 0x0f),
    shortcut("add_marker", ["Add Marker", "Add Marker", "Add Marker", "Add Marker"], 0x00, 0x10),
    shortcut("play_stop", ["Play/Stop", "Play/Stop", "Play/Stop", "Play/Stop"], 0x00, 0x2c),
    shortcut("open_color_page", ["Color Page", "Color Page", "Color Page", "Color Page"], 0x02, 0x23),
    shortcut("loop_playback", ["Loop", "Loop", "Loop", "Loop"], 0x08, 0x38),
    shortcut("cinema_viewer", ["Cinema Viewer", "Cinema Viewer", "Cinema Viewer", "Cinema Viewer"], 0x08, 0x09),
];

const FINALCUTPRO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("blade", ["Blade", "Blade", "Blade", "Blade"], 0x08, 0x05),
    shortcut("append_to_storyline", ["Append to Storyline", "Append to Storyline", "Append to Storyline", "Append to Storyline"], 0x00, 0x08),
    shortcut("connect_to_storyline", ["Connect to Primary Storyline", "Connect to Primary Storyline", "Connect to Primary Storyline", "Connect to Primary Storyline"], 0x00, 0x14),
    shortcut("insert_edit", ["Insert", "Insert", "Insert", "Insert"], 0x00, 0x1a),
    shortcut("overwrite_edit", ["Overwrite", "Overwrite", "Overwrite", "Overwrite"], 0x00, 0x07),
    shortcut("add_marker", ["Add Marker", "Add Marker", "Add Marker", "Add Marker"], 0x00, 0x10),
    shortcut("select_tool", ["Select", "Select", "Select", "Select"], 0x00, 0x04),
    shortcut("trim_tool", ["Trim", "Trim", "Trim", "Trim"], 0x00, 0x17),
    shortcut("show_retime_editor", ["Show Retime Editor", "Show Retime Editor", "Show Retime Editor", "Show Retime Editor"], 0x08, 0x15),
    shortcut("play_pause", ["Play/Pause", "Play/Pause", "Play/Pause", "Play/Pause"], 0x00, 0x2c),
];

const PREMIEREPRO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("add_edit", ["Add Edit", "Add Edit", "Add Edit", "Add Edit"], 0x08, 0x0e),
    shortcut("add_marker", ["Add Marker", "Add Marker", "Add Marker", "Add Marker"], 0x00, 0x10),
    shortcut("insert", ["Insert", "Insert", "Insert", "Insert"], 0x00, 0x36),
    shortcut("overwrite", ["Overwrite", "Overwrite", "Overwrite", "Overwrite"], 0x00, 0x37),
    shortcut("lift", ["Lift", "Lift", "Lift", "Lift"], 0x00, 0x33),
    shortcut("extract", ["Extract", "Extract", "Extract", "Extract"], 0x00, 0x34),
    shortcut("razor_tool", ["Razor Tool", "Razor Tool", "Razor Tool", "Razor Tool"], 0x00, 0x06),
    shortcut("selection_tool", ["Selection Tool", "Selection Tool", "Selection Tool", "Selection Tool"], 0x00, 0x19),
    shortcut("match_frame", ["Match Frame", "Match Frame", "Match Frame", "Match Frame"], 0x00, 0x09),
    shortcut("export_media", ["Export Media", "Export Media", "Export Media", "Export Media"], 0x08, 0x10),
];

const OBSSTUDIO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("edit_transform", ["Edit Transform", "Edit Transform", "Edit Transform", "Edit Transform"], 0x08, 0x08),
    shortcut("fit_to_screen", ["Fit to Screen", "Fit to Screen", "Fit to Screen", "Fit to Screen"], 0x08, 0x09),
    shortcut("stretch_to_screen", ["Stretch to Screen", "Stretch to Screen", "Stretch to Screen", "Stretch to Screen"], 0x08, 0x16),
    shortcut("center_to_screen", ["Center to Screen", "Center to Screen", "Center to Screen", "Center to Screen"], 0x08, 0x07),
    shortcut("reset_transform", ["Reset Transform", "Reset Transform", "Reset Transform", "Reset Transform"], 0x08, 0x15),
    shortcut("move_source_up", ["Move Source Up", "Move Source Up", "Move Source Up", "Move Source Up"], 0x08, 0x52),
    shortcut("move_source_down", ["Move Source Down", "Move Source Down", "Move Source Down", "Move Source Down"], 0x08, 0x51),
];

const PHOTOSHOP_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("free_transform", ["Free Transform", "Free Transform", "Free Transform", "Free Transform"], 0x08, 0x17),
    shortcut("layer_via_copy", ["Layer Via Copy", "Layer Via Copy", "Layer Via Copy", "Layer Via Copy"], 0x08, 0x0d),
    shortcut("new_layer", ["New Layer", "New Layer", "New Layer", "New Layer"], 0x0a, 0x11),
    shortcut("deselect", ["Deselect", "Deselect", "Deselect", "Deselect"], 0x08, 0x07),
    shortcut("select_inverse", ["Inverse", "Inverse", "Inverse", "Inverse"], 0x0a, 0x0c),
    shortcut("increase_brush_size", ["Increase Brush Size", "Increase Brush Size", "Increase Brush Size", "Increase Brush Size"], 0x00, 0x30),
    shortcut("decrease_brush_size", ["Decrease Brush Size", "Decrease Brush Size", "Decrease Brush Size", "Decrease Brush Size"], 0x00, 0x2f),
    shortcut("merge_visible", ["Merge Visible", "Merge Visible", "Merge Visible", "Merge Visible"], 0x0a, 0x08),
    shortcut("fit_on_screen", ["Fit on Screen", "Fit on Screen", "Fit on Screen", "Fit on Screen"], 0x08, 0x27),
    shortcut("brush_tool", ["Brush Tool", "Brush Tool", "Brush Tool", "Brush Tool"], 0x00, 0x05),
];

const ILLUSTRATOR_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("transform_again", ["Transform Again", "Transform Again", "Transform Again", "Transform Again"], 0x08, 0x07),
    shortcut("group", ["Group", "Group", "Group", "Group"], 0x08, 0x0a),
    shortcut("ungroup", ["Ungroup", "Ungroup", "Ungroup", "Ungroup"], 0x0a, 0x0a),
    shortcut("make_clipping_mask", ["Make Clipping Mask", "Make Clipping Mask", "Make Clipping Mask", "Make Clipping Mask"], 0x08, 0x24),
    shortcut("lock_selection", ["Lock Selection", "Lock Selection", "Lock Selection", "Lock Selection"], 0x08, 0x1f),
    shortcut("toggle_outline_mode", ["Outline", "Outline", "Outline", "Outline"], 0x08, 0x1c),
    shortcut("create_outlines", ["Create Outlines", "Create Outlines", "Create Outlines", "Create Outlines"], 0x0a, 0x12),
    shortcut("bring_to_front", ["Bring to Front", "Bring to Front", "Bring to Front", "Bring to Front"], 0x0a, 0x30),
    shortcut("send_to_back", ["Send to Back", "Send to Back", "Send to Back", "Send to Back"], 0x0a, 0x2f),
    shortcut("fit_artboard_in_window", ["Fit Artboard in Window", "Fit Artboard in Window", "Fit Artboard in Window", "Fit Artboard in Window"], 0x08, 0x27),
];

const LOGICPRO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_stop", ["Play or Stop", "Play or Stop", "Play or Stop", "Play or Stop"], 0x00, 0x2c),
    shortcut("record", ["Record", "Record", "Record", "Record"], 0x00, 0x15),
    shortcut("cycle_mode", ["Cycle Mode", "Cycle Mode", "Cycle Mode", "Cycle Mode"], 0x00, 0x06),
    shortcut("metronome", ["Metronome", "Metronome", "Metronome", "Metronome"], 0x00, 0x0e),
    shortcut("go_to_beginning", ["Go to Beginning", "Go to Beginning", "Go to Beginning", "Go to Beginning"], 0x00, 0x28),
    shortcut("show_mixer", ["Show Mixer", "Show Mixer", "Show Mixer", "Show Mixer"], 0x00, 0x1b),
    shortcut("show_editors", ["Show Editors", "Show Editors", "Show Editors", "Show Editors"], 0x00, 0x08),
    shortcut("show_library", ["Show Library", "Show Library", "Show Library", "Show Library"], 0x00, 0x1c),
    shortcut("show_automation", ["Show Automation", "Show Automation", "Show Automation", "Show Automation"], 0x00, 0x04),
    shortcut("bounce_project", ["Bounce Project or Section", "Bounce Project or Section", "Bounce Project or Section", "Bounce Project or Section"], 0x08, 0x05),
];

const BLENDER_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_edit_mode", ["Toggle Edit Mode", "Toggle Edit Mode", "Toggle Edit Mode", "Toggle Edit Mode"], 0x00, 0x2b),
    shortcut("play_animation", ["Play Animation", "Play Animation", "Play Animation", "Play Animation"], 0x00, 0x2c),
    shortcut("shading_pie_menu", ["Shading Pie Menu", "Shading Pie Menu", "Shading Pie Menu", "Shading Pie Menu"], 0x00, 0x1d),
    shortcut("add_menu", ["Add", "Add", "Add", "Add"], 0x02, 0x04),
    shortcut("duplicate_objects", ["Duplicate Objects", "Duplicate Objects", "Duplicate Objects", "Duplicate Objects"], 0x02, 0x07),
    shortcut("frame_all", ["Frame All", "Frame All", "Frame All", "Frame All"], 0x00, 0x4a),
    shortcut("toggle_sidebar", ["Toggle Sidebar", "Toggle Sidebar", "Toggle Sidebar", "Toggle Sidebar"], 0x00, 0x11),
    shortcut("toggle_toolbar", ["Toggle Toolbar", "Toggle Toolbar", "Toggle Toolbar", "Toggle Toolbar"], 0x00, 0x17),
    shortcut("render_image", ["Render Image", "Render Image", "Render Image", "Render Image"], 0x00, 0x45),
    shortcut("maximize_area", ["Toggle Maximize Area", "Toggle Maximize Area", "Toggle Maximize Area", "Toggle Maximize Area"], 0x01, 0x2c),
];

const ITERM2_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("split_vertically", ["Split Vertically", "Split Vertically", "Split Vertically", "Split Vertically"], 0x08, 0x07),
    shortcut("split_horizontally", ["Split Horizontally", "Split Horizontally", "Split Horizontally", "Split Horizontally"], 0x0a, 0x07),
    shortcut("maximize_pane", ["Maximize Active Pane", "Maximize Active Pane", "Maximize Active Pane", "Maximize Active Pane"], 0x0a, 0x28),
    shortcut("clear_buffer", ["Clear Buffer", "Clear Buffer", "Clear Buffer", "Clear Buffer"], 0x08, 0x0e),
    shortcut("broadcast_input_tab", ["Broadcast Input to Tab", "Broadcast Input to Tab", "Broadcast Input to Tab", "Broadcast Input to Tab"], 0x0c, 0x0c),
    shortcut("instant_replay", ["Start Instant Replay", "Start Instant Replay", "Start Instant Replay", "Start Instant Replay"], 0x0c, 0x05),
    shortcut("paste_history", ["Open Paste History", "Open Paste History", "Open Paste History", "Open Paste History"], 0x0a, 0x0b),
    shortcut("set_mark", ["Set Mark", "Set Mark", "Set Mark", "Set Mark"], 0x0a, 0x10),
    shortcut("open_autocomplete", ["Open Autocomplete", "Open Autocomplete", "Open Autocomplete", "Open Autocomplete"], 0x08, 0x33),
];

const INTELLIJ_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("run", ["Run", "Run", "Run", "Run"], 0x01, 0x15),
    shortcut("debug", ["Debug", "Debug", "Debug", "Debug"], 0x01, 0x07),
    shortcut("find_action", ["Find Action", "Find Action", "Find Action", "Find Action"], 0x0a, 0x04),
    shortcut("recent_files", ["Recent Files", "Recent Files", "Recent Files", "Recent Files"], 0x08, 0x08),
    shortcut("reformat_code", ["Reformat Code", "Reformat Code", "Reformat Code", "Reformat Code"], 0x0c, 0x0f),
    shortcut("rename", ["Rename", "Rename", "Rename", "Rename"], 0x02, 0x3f),
    shortcut("show_context_actions", ["Show Context Actions", "Show Context Actions", "Show Context Actions", "Show Context Actions"], 0x04, 0x28),
    shortcut("build_project", ["Build Project", "Build Project", "Build Project", "Build Project"], 0x08, 0x42),
    shortcut("step_over", ["Step Over", "Step Over", "Step Over", "Step Over"], 0x00, 0x41),
    shortcut("hide_all_tool_windows", ["Hide All Tool Windows", "Hide All Tool Windows", "Hide All Tool Windows", "Hide All Tool Windows"], 0x0a, 0x45),
];

const CURSOR_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("open_agent", ["Open Agent", "Open Agent", "Open Agent", "Open Agent"], 0x08, 0x0c),
    shortcut("toggle_chat", ["Toggle Chat", "Toggle Chat", "Toggle Chat", "Toggle Chat"], 0x08, 0x0f),
    shortcut("inline_edit", ["Inline Edit", "Inline Edit", "Inline Edit", "Inline Edit"], 0x08, 0x0e),
    shortcut("add_selection_to_chat", ["Add Selection to Chat", "Add Selection to Chat", "Add Selection to Chat", "Add Selection to Chat"], 0x0a, 0x0f),
    shortcut("cursor_settings", ["Cursor Settings", "Cursor Settings", "Cursor Settings", "Cursor Settings"], 0x0a, 0x0d),
    shortcut("toggle_terminal", ["Toggle Terminal", "Toggle Terminal", "Toggle Terminal", "Toggle Terminal"], 0x01, 0x35),
    shortcut("toggle_sidebar", ["Toggle Primary Side Bar", "Toggle Primary Side Bar", "Toggle Primary Side Bar", "Toggle Primary Side Bar"], 0x08, 0x05),
    shortcut("command_palette", ["Command Palette", "Command Palette", "Command Palette", "Command Palette"], 0x0a, 0x13),
    shortcut("toggle_panel", ["Toggle Panel", "Toggle Panel", "Toggle Panel", "Toggle Panel"], 0x08, 0x0d),
];

const FIREFOX_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("undo_close_tab", ["Undo Close Tab", "Undo Close Tab", "Undo Close Tab", "Undo Close Tab"], 0x0a, 0x17),
    shortcut("reader_view", ["Toggle Reader View", "Toggle Reader View", "Toggle Reader View", "Toggle Reader View"], 0x0c, 0x15),
    shortcut("mute_tab", ["Mute/Unmute Audio", "Mute/Unmute Audio", "Mute/Unmute Audio", "Mute/Unmute Audio"], 0x01, 0x10),
    shortcut("private_window", ["New Private Window", "New Private Window", "New Private Window", "New Private Window"], 0x0a, 0x13),
    shortcut("downloads", ["Downloads", "Downloads", "Downloads", "Downloads"], 0x08, 0x0d),
    shortcut("bookmarks_sidebar", ["Bookmarks Sidebar", "Bookmarks Sidebar", "Bookmarks Sidebar", "Bookmarks Sidebar"], 0x08, 0x05),
    shortcut("history_sidebar", ["History Sidebar", "History Sidebar", "History Sidebar", "History Sidebar"], 0x0a, 0x0b),
    shortcut("focus_address_bar", ["Open Location", "Open Location", "Open Location", "Open Location"], 0x08, 0x0f),
    shortcut("next_tab", ["Next Tab", "Next Tab", "Next Tab", "Next Tab"], 0x01, 0x2b),
];

const ARC_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_sidebar", ["Toggle Sidebar", "Toggle Sidebar", "Toggle Sidebar", "Toggle Sidebar"], 0x08, 0x16),
    shortcut("copy_url", ["Copy URL", "Copy URL", "Copy URL", "Copy URL"], 0x0a, 0x06),
    shortcut("little_arc", ["New Little Arc Window", "New Little Arc Window", "New Little Arc Window", "New Little Arc Window"], 0x0c, 0x11),
    shortcut("next_space", ["Next Space", "Next Space", "Next Space", "Next Space"], 0x0c, 0x4f),
    shortcut("previous_space", ["Previous Space", "Previous Space", "Previous Space", "Previous Space"], 0x0c, 0x50),
    shortcut("next_tab", ["Next Tab", "Next Tab", "Next Tab", "Next Tab"], 0x0c, 0x51),
    shortcut("previous_tab", ["Previous Tab", "Previous Tab", "Previous Tab", "Previous Tab"], 0x0c, 0x52),
    shortcut("restore_tab", ["Restore Closed Tab", "Restore Closed Tab", "Restore Closed Tab", "Restore Closed Tab"], 0x0a, 0x17),
    shortcut("new_tab", ["New Tab", "New Tab", "New Tab", "New Tab"], 0x08, 0x17),
];

const NOTES_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_note", ["New Note", "New Note", "New Note", "New Note"], 0x08, 0x11),
    shortcut("checklist", ["Checklist", "Checklist", "Checklist", "Checklist"], 0x0a, 0x0f),
    shortcut("mark_checklist_item", ["Mark as Checked", "Mark as Checked", "Mark as Checked", "Mark as Checked"], 0x0a, 0x18),
    shortcut("format_title", ["Title", "Title", "Title", "Title"], 0x0a, 0x17),
    shortcut("format_heading", ["Heading", "Heading", "Heading", "Heading"], 0x0a, 0x0b),
    shortcut("format_body", ["Body", "Body", "Body", "Body"], 0x0a, 0x05),
    shortcut("insert_table", ["Table", "Table", "Table", "Table"], 0x0c, 0x17),
    shortcut("search_all_notes", ["Note List Search", "Note List Search", "Note List Search", "Note List Search"], 0x0c, 0x09),
];

const MAIL_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("archive", ["Archive", "Archive", "Archive", "Archive"], 0x09, 0x04),
    shortcut("reply", ["Reply", "Reply", "Reply", "Reply"], 0x08, 0x15),
    shortcut("reply_all", ["Reply All", "Reply All", "Reply All", "Reply All"], 0x0a, 0x15),
    shortcut("forward", ["Forward", "Forward", "Forward", "Forward"], 0x0a, 0x09),
    shortcut("send", ["Send", "Send", "Send", "Send"], 0x0a, 0x07),
    shortcut("mark_read_unread", ["Mark as Read/Unread", "Mark as Read/Unread", "Mark as Read/Unread", "Mark as Read/Unread"], 0x0a, 0x18),
    shortcut("flag", ["Toggle Flag", "Toggle Flag", "Toggle Flag", "Toggle Flag"], 0x0a, 0x0f),
    shortcut("get_new_mail", ["Get New Mail", "Get New Mail", "Get New Mail", "Get New Mail"], 0x0a, 0x11),
    shortcut("move_to_junk", ["Move to Junk", "Move to Junk", "Move to Junk", "Move to Junk"], 0x0a, 0x0d),
    shortcut("new_message", ["New Message", "New Message", "New Message", "New Message"], 0x08, 0x11),
];

const KEYNOTE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_slideshow", ["Play Slideshow", "Play Slideshow", "Play Slideshow", "Play Slideshow"], 0x0c, 0x13),
    shortcut("presenter_notes", ["Show Presenter Notes", "Show Presenter Notes", "Show Presenter Notes", "Show Presenter Notes"], 0x0a, 0x13),
    shortcut("new_slide", ["New Slide", "New Slide", "New Slide", "New Slide"], 0x0a, 0x11),
    shortcut("skip_slide", ["Skip Slide", "Skip Slide", "Skip Slide", "Skip Slide"], 0x0a, 0x0b),
    shortcut("add_comment", ["Comment", "Comment", "Comment", "Comment"], 0x0a, 0x0e),
    shortcut("group_objects", ["Group", "Group", "Group", "Group"], 0x0c, 0x0a),
    shortcut("ungroup_objects", ["Ungroup", "Ungroup", "Ungroup", "Ungroup"], 0x0e, 0x0a),
    shortcut("toggle_inspector", ["Show/Hide Inspector", "Show/Hide Inspector", "Show/Hide Inspector", "Show/Hide Inspector"], 0x0c, 0x0c),
];

const PREVIEW_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("markup_toolbar", ["Show Markup Toolbar", "Show Markup Toolbar", "Show Markup Toolbar", "Show Markup Toolbar"], 0x0a, 0x04),
    shortcut("highlight_text", ["Highlight Text", "Highlight Text", "Highlight Text", "Highlight Text"], 0x09, 0x0b),
    shortcut("rotate_left", ["Rotate Left", "Rotate Left", "Rotate Left", "Rotate Left"], 0x08, 0x0f),
    shortcut("rotate_right", ["Rotate Right", "Rotate Right", "Rotate Right", "Rotate Right"], 0x08, 0x15),
    shortcut("thumbnails", ["Thumbnails", "Thumbnails", "Thumbnails", "Thumbnails"], 0x0c, 0x1f),
    shortcut("table_of_contents", ["Table of Contents", "Table of Contents", "Table of Contents", "Table of Contents"], 0x0c, 0x20),
    shortcut("show_inspector", ["Show Inspector", "Show Inspector", "Show Inspector", "Show Inspector"], 0x08, 0x0c),
    shortcut("zoom_to_fit", ["Zoom to Fit", "Zoom to Fit", "Zoom to Fit", "Zoom to Fit"], 0x08, 0x26),
];

const NOTION_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_sidebar", ["Toggle Sidebar", "Toggle Sidebar", "Toggle Sidebar", "Toggle Sidebar"], 0x08, 0x31),
    shortcut("search", ["Search", "Search", "Search", "Search"], 0x08, 0x13),
    shortcut("toggle_dark_mode", ["Toggle Dark Mode", "Toggle Dark Mode", "Toggle Dark Mode", "Toggle Dark Mode"], 0x0a, 0x0f),
    shortcut("go_back", ["Go Back", "Go Back", "Go Back", "Go Back"], 0x08, 0x2f),
    shortcut("go_forward", ["Go Forward", "Go Forward", "Go Forward", "Go Forward"], 0x08, 0x30),
    shortcut("new_page", ["New Page", "New Page", "New Page", "New Page"], 0x08, 0x11),
    shortcut("new_tab", ["New Tab", "New Tab", "New Tab", "New Tab"], 0x08, 0x17),
    shortcut("comment", ["Comment", "Comment", "Comment", "Comment"], 0x0a, 0x10),
    shortcut("new_window", ["New Window", "New Window", "New Window", "New Window"], 0x0a, 0x11),
];

const OBSIDIAN_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("command_palette", ["Open Command Palette", "Open Command Palette", "Open Command Palette", "Open Command Palette"], 0x08, 0x13),
    shortcut("quick_switcher", ["Open Quick Switcher", "Open Quick Switcher", "Open Quick Switcher", "Open Quick Switcher"], 0x08, 0x12),
    shortcut("toggle_reading_view", ["Toggle Reading View", "Toggle Reading View", "Toggle Reading View", "Toggle Reading View"], 0x08, 0x08),
    shortcut("graph_view", ["Open Graph View", "Open Graph View", "Open Graph View", "Open Graph View"], 0x08, 0x0a),
    shortcut("global_search", ["Search in All Files", "Search in All Files", "Search in All Files", "Search in All Files"], 0x0a, 0x09),
    shortcut("new_note", ["Create New Note", "Create New Note", "Create New Note", "Create New Note"], 0x08, 0x11),
    shortcut("toggle_checkbox", ["Toggle Checkbox Status", "Toggle Checkbox Status", "Toggle Checkbox Status", "Toggle Checkbox Status"], 0x08, 0x0f),
    shortcut("navigate_back", ["Navigate Back", "Navigate Back", "Navigate Back", "Navigate Back"], 0x0c, 0x50),
    shortcut("navigate_forward", ["Navigate Forward", "Navigate Forward", "Navigate Forward", "Navigate Forward"], 0x0c, 0x4f),
];

const ONEPASSWORD_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_access", ["Show Quick Access", "Show Quick Access", "Show Quick Access", "Show Quick Access"], 0x0a, 0x2c),
    shortcut("autofill", ["Autofill", "Autofill", "Autofill", "Autofill"], 0x08, 0x31),
    shortcut("lock", ["Lock 1Password", "Lock 1Password", "Lock 1Password", "Lock 1Password"], 0x0a, 0x0f),
    shortcut("copy_password", ["Copy Password", "Copy Password", "Copy Password", "Copy Password"], 0x0a, 0x06),
    shortcut("copy_one_time_password", ["Copy One-Time Password", "Copy One-Time Password", "Copy One-Time Password", "Copy One-Time Password"], 0x0c, 0x06),
    shortcut("open_and_fill", ["Open and Fill", "Open and Fill", "Open and Fill", "Open and Fill"], 0x0a, 0x09),
    shortcut("new_item", ["New Item", "New Item", "New Item", "New Item"], 0x08, 0x11),
    shortcut("edit_item", ["Edit Item", "Edit Item", "Edit Item", "Edit Item"], 0x08, 0x08),
    shortcut("toggle_sidebar", ["Collapse/Expand Sidebar", "Collapse/Expand Sidebar", "Collapse/Expand Sidebar", "Collapse/Expand Sidebar"], 0x0a, 0x07),
];

const SPOTIFY_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_pause", ["Play/Pause", "Play/Pause", "Play/Pause", "Play/Pause"], 0x00, 0x2c),
    shortcut("next_track", ["Next Track", "Next Track", "Next Track", "Next Track"], 0x08, 0x4f),
    shortcut("previous_track", ["Previous Track", "Previous Track", "Previous Track", "Previous Track"], 0x08, 0x50),
    shortcut("volume_up", ["Volume Up", "Volume Up", "Volume Up", "Volume Up"], 0x08, 0x52),
    shortcut("volume_down", ["Volume Down", "Volume Down", "Volume Down", "Volume Down"], 0x08, 0x51),
    shortcut("toggle_shuffle", ["Shuffle", "Shuffle", "Shuffle", "Shuffle"], 0x04, 0x16),
    shortcut("toggle_repeat", ["Repeat", "Repeat", "Repeat", "Repeat"], 0x04, 0x15),
    shortcut("like_song", ["Like/Dislike Song", "Like/Dislike Song", "Like/Dislike Song", "Like/Dislike Song"], 0x06, 0x05),
    shortcut("go_to_queue", ["Go to Queue", "Go to Queue", "Go to Queue", "Go to Queue"], 0x06, 0x14),
    shortcut("open_search", ["Open Search", "Open Search", "Open Search", "Open Search"], 0x08, 0x0e),
];

const MUSIC_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_pause", ["Play/Pause", "Play/Pause", "Play/Pause", "Play/Pause"], 0x00, 0x2c),
    shortcut("next_song", ["Next Song", "Next Song", "Next Song", "Next Song"], 0x00, 0x4f),
    shortcut("previous_song", ["Previous Song", "Previous Song", "Previous Song", "Previous Song"], 0x00, 0x50),
    shortcut("volume_up", ["Turn Volume Up", "Turn Volume Up", "Turn Volume Up", "Turn Volume Up"], 0x08, 0x52),
    shortcut("volume_down", ["Turn Volume Down", "Turn Volume Down", "Turn Volume Down", "Turn Volume Down"], 0x08, 0x51),
    shortcut("miniplayer", ["MiniPlayer", "MiniPlayer", "MiniPlayer", "MiniPlayer"], 0x0a, 0x10),
    shortcut("playing_next", ["Playing Next", "Playing Next", "Playing Next", "Playing Next"], 0x0c, 0x18),
    shortcut("visualizer", ["Visualizer", "Visualizer", "Visualizer", "Visualizer"], 0x08, 0x17),
    shortcut("equalizer", ["Equalizer", "Equalizer", "Equalizer", "Equalizer"], 0x0c, 0x08),
];

const DISCORD_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_mute", ["Toggle Mute", "Toggle Mute", "Toggle Mute", "Toggle Mute"], 0x0a, 0x10),
    shortcut("toggle_deafen", ["Toggle Deafen", "Toggle Deafen", "Toggle Deafen", "Toggle Deafen"], 0x0a, 0x07),
    shortcut("quick_switcher", ["Quick Switcher", "Quick Switcher", "Quick Switcher", "Quick Switcher"], 0x08, 0x0e),
    shortcut("mark_server_read", ["Mark Server as Read", "Mark Server as Read", "Mark Server as Read", "Mark Server as Read"], 0x02, 0x29),
    shortcut("toggle_pins", ["Toggle Pins", "Toggle Pins", "Toggle Pins", "Toggle Pins"], 0x08, 0x13),
    shortcut("toggle_inbox", ["Toggle Inbox", "Toggle Inbox", "Toggle Inbox", "Toggle Inbox"], 0x08, 0x0c),
    shortcut("emoji_picker", ["Toggle Emoji Picker", "Toggle Emoji Picker", "Toggle Emoji Picker", "Toggle Emoji Picker"], 0x08, 0x08),
    shortcut("gif_picker", ["Toggle GIF Picker", "Toggle GIF Picker", "Toggle GIF Picker", "Toggle GIF Picker"], 0x08, 0x0a),
    shortcut("upload_file", ["Upload a File", "Upload a File", "Upload a File", "Upload a File"], 0x0a, 0x18),
];

pub const APPLICATION_SHORTCUTS: &[ShortcutApplication] = &[
    ShortcutApplication {
        id: "finder",
        label: "Finder",
        icon: "app-window-mac",
        shortcuts: FINDER_SHORTCUTS,
    },
    ShortcutApplication {
        id: "safari",
        label: "Safari",
        icon: "simple:safari",
        shortcuts: SAFARI_SHORTCUTS,
    },
    ShortcutApplication {
        id: "chrome",
        label: "Google Chrome",
        icon: "simple:googlechrome",
        shortcuts: CHROME_SHORTCUTS,
    },
    ShortcutApplication {
        id: "vscode",
        label: "Visual Studio Code",
        icon: "code-xml",
        shortcuts: VSCODE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "xcode",
        label: "Xcode",
        icon: "simple:xcode",
        shortcuts: XCODE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "terminal",
        label: "Terminal",
        icon: "square-terminal",
        shortcuts: TERMINAL_SHORTCUTS,
    },
    ShortcutApplication {
        id: "slack",
        label: "Slack",
        icon: "hash",
        shortcuts: SLACK_SHORTCUTS,
    },
    ShortcutApplication {
        id: "figma",
        label: "Figma",
        icon: "simple:figma",
        shortcuts: FIGMA_SHORTCUTS,
    },
    ShortcutApplication {
        id: "zoom",
        label: "Zoom Workplace",
        icon: "simple:zoom",
        shortcuts: ZOOM_SHORTCUTS,
    },
    ShortcutApplication {
        id: "davinciresolve",
        label: "DaVinci Resolve",
        icon: "simple:davinciresolve",
        shortcuts: DAVINCIRESOLVE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "finalcutpro",
        label: "Final Cut Pro",
        icon: "clapperboard",
        shortcuts: FINALCUTPRO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "premierepro",
        label: "Premiere Pro",
        icon: "film",
        shortcuts: PREMIEREPRO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "obsstudio",
        label: "OBS Studio",
        icon: "simple:obsstudio",
        shortcuts: OBSSTUDIO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "photoshop",
        label: "Adobe Photoshop",
        icon: "image",
        shortcuts: PHOTOSHOP_SHORTCUTS,
    },
    ShortcutApplication {
        id: "illustrator",
        label: "Adobe Illustrator",
        icon: "pen-tool",
        shortcuts: ILLUSTRATOR_SHORTCUTS,
    },
    ShortcutApplication {
        id: "logicpro",
        label: "Logic Pro",
        icon: "music",
        shortcuts: LOGICPRO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "blender",
        label: "Blender",
        icon: "simple:blender",
        shortcuts: BLENDER_SHORTCUTS,
    },
    ShortcutApplication {
        id: "iterm2",
        label: "iTerm2",
        icon: "simple:iterm2",
        shortcuts: ITERM2_SHORTCUTS,
    },
    ShortcutApplication {
        id: "intellij",
        label: "IntelliJ IDEA",
        icon: "simple:intellijidea",
        shortcuts: INTELLIJ_SHORTCUTS,
    },
    ShortcutApplication {
        id: "cursor",
        label: "Cursor",
        icon: "simple:cursor",
        shortcuts: CURSOR_SHORTCUTS,
    },
    ShortcutApplication {
        id: "firefox",
        label: "Firefox",
        icon: "simple:firefox",
        shortcuts: FIREFOX_SHORTCUTS,
    },
    ShortcutApplication {
        id: "arc",
        label: "Arc",
        icon: "simple:arc",
        shortcuts: ARC_SHORTCUTS,
    },
    ShortcutApplication {
        id: "notes",
        label: "Apple Notes",
        icon: "notebook",
        shortcuts: NOTES_SHORTCUTS,
    },
    ShortcutApplication {
        id: "mail",
        label: "Apple Mail",
        icon: "mail",
        shortcuts: MAIL_SHORTCUTS,
    },
    ShortcutApplication {
        id: "keynote",
        label: "Apple Keynote",
        icon: "presentation",
        shortcuts: KEYNOTE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "preview",
        label: "Apple Preview",
        icon: "file-image",
        shortcuts: PREVIEW_SHORTCUTS,
    },
    ShortcutApplication {
        id: "notion",
        label: "Notion",
        icon: "simple:notion",
        shortcuts: NOTION_SHORTCUTS,
    },
    ShortcutApplication {
        id: "obsidian",
        label: "Obsidian",
        icon: "simple:obsidian",
        shortcuts: OBSIDIAN_SHORTCUTS,
    },
    ShortcutApplication {
        id: "onepassword",
        label: "1Password",
        icon: "simple:1password",
        shortcuts: ONEPASSWORD_SHORTCUTS,
    },
    ShortcutApplication {
        id: "spotify",
        label: "Spotify",
        icon: "simple:spotify",
        shortcuts: SPOTIFY_SHORTCUTS,
    },
    ShortcutApplication {
        id: "music",
        label: "Music",
        icon: "simple:applemusic",
        shortcuts: MUSIC_SHORTCUTS,
    },
    ShortcutApplication {
        id: "discord",
        label: "Discord",
        icon: "simple:discord",
        shortcuts: DISCORD_SHORTCUTS,
    },
];
// --- generated shortcut catalog end ---

const fn shortcut(
    id: &'static str,
    labels: [&'static str; 4],
    mods: u8,
    key: u16,
) -> ShortcutPreset {
    ShortcutPreset {
        id,
        labels,
        mods,
        key,
    }
}

pub fn shortcut_application(id: &str) -> Option<&'static ShortcutApplication> {
    APPLICATION_SHORTCUTS.iter().find(|app| app.id == id)
}

pub fn shortcut_preset(application: &str, id: &str) -> Option<&'static ShortcutPreset> {
    shortcut_application(application)?
        .shortcuts
        .iter()
        .find(|shortcut| shortcut.id == id)
}

pub fn shortcut_chord_label(shortcut: &ShortcutPreset) -> String {
    let mods = mods_label(shortcut.mods);
    let key = keyboard_name(shortcut.key).unwrap_or("Unknown key");
    if mods.is_empty() {
        key.to_string()
    } else {
        format!("{mods} + {key}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacOsPreset {
    pub command: MacOsControl,
    pub label: &'static str,
    pub detail: &'static str,
}

pub const MACOS_PRESETS: &[MacOsPreset] = &[
    macos(
        MacOsControl::BrightnessUp,
        "Brightness up",
        "Increase the built-in display brightness.",
    ),
    macos(
        MacOsControl::BrightnessDown,
        "Brightness down",
        "Decrease the built-in display brightness.",
    ),
    macos(
        MacOsControl::MissionControl,
        "Mission Control",
        "Show all open windows and spaces.",
    ),
    macos(
        MacOsControl::Applications,
        "Applications / Launchpad",
        "Show the macOS applications view.",
    ),
    macos(
        MacOsControl::Search,
        "Spotlight Search",
        "Open or close Spotlight.",
    ),
    macos(
        MacOsControl::Dictation,
        "Dictation",
        "Start or stop macOS Dictation.",
    ),
    macos(
        MacOsControl::Globe,
        "Globe / Fn",
        "Use the native Globe key for input switching and Globe shortcuts.",
    ),
    macos(
        MacOsControl::LockScreen,
        "Lock screen",
        "Lock the current macOS session.",
    ),
    macos(MacOsControl::Sleep, "Sleep", "Put this Mac to sleep."),
    macos(
        MacOsControl::VolumeUp,
        "Volume up",
        "Increase the system output volume.",
    ),
    macos(
        MacOsControl::VolumeDown,
        "Volume down",
        "Decrease the system output volume.",
    ),
    macos(MacOsControl::Mute, "Mute", "Toggle system audio mute."),
    macos(
        MacOsControl::PlayPause,
        "Play / pause",
        "Toggle media playback.",
    ),
    macos(
        MacOsControl::NextTrack,
        "Next track",
        "Skip to the next media item.",
    ),
    macos(
        MacOsControl::PreviousTrack,
        "Previous track",
        "Return to the previous media item.",
    ),
    macos(
        MacOsControl::EmojiPicker,
        "Emoji & symbols",
        "Open or close the character picker.",
    ),
];

const fn macos(command: MacOsControl, label: &'static str, detail: &'static str) -> MacOsPreset {
    MacOsPreset {
        command,
        label,
        detail,
    }
}

pub fn macos_preset(command: MacOsControl) -> &'static MacOsPreset {
    MACOS_PRESETS
        .iter()
        .find(|preset| preset.command == command)
        .unwrap_or(&MACOS_PRESETS[0])
}

/// Apply a curated application chord. Returns false only for stale/unknown
/// catalog IDs, leaving the existing mapping untouched.
pub fn apply_application_shortcut(
    input: &mut InputConfig,
    application: &str,
    shortcut_id: &str,
) -> bool {
    let Some(shortcut) = shortcut_preset(application, shortcut_id) else {
        return false;
    };
    input.behavior = Some(ControlBehavior::ApplicationShortcut {
        application: application.to_string(),
        shortcut: shortcut_id.to_string(),
    });
    input.emitted = keyboard_slot(shortcut.mods, shortcut.key);
    input.action = Action::None;
    true
}

pub fn apply_keystroke(input: &mut InputConfig, mods: u8, key: u16) {
    input.behavior = Some(ControlBehavior::Keystroke);
    input.emitted = keyboard_slot(mods & 0x0f, key);
    input.action = Action::None;
}

pub fn apply_macos(input: &mut InputConfig, slot_index: usize, command: MacOsControl) {
    input.behavior = Some(ControlBehavior::MacOs { command });
    let (emitted, action) = match command {
        MacOsControl::BrightnessUp => (consumer_slot(0x006F), Action::None),
        MacOsControl::BrightnessDown => (consumer_slot(0x0070), Action::None),
        MacOsControl::MissionControl => (keyboard_slot(0x01, 0x52), Action::None),
        MacOsControl::Applications => {
            let target = if Path::new("/System/Applications/Apps.app").exists() {
                "/System/Applications/Apps.app"
            } else {
                "/System/Applications/Launchpad.app"
            };
            (
                host_trigger(slot_index),
                Action::Open {
                    target: target.to_string(),
                },
            )
        }
        // The documented macOS default. Users can remap Spotlight in System
        // Settings, just like any other keyboard shortcut.
        MacOsControl::Search => (keyboard_slot(0x08, 0x2C), Action::None),
        MacOsControl::Dictation => (consumer_slot(0x00D8), Action::None),
        // Apple's accessory keyboard specification assigns the native Globe
        // key to Consumer-page AC Keyboard Layout Select (0x029D).
        MacOsControl::Globe => (consumer_slot(0x029D), Action::None),
        MacOsControl::LockScreen => (keyboard_slot(0x09, 0x14), Action::None),
        MacOsControl::Sleep => (
            host_trigger(slot_index),
            Action::Run {
                command: "pmset sleepnow".to_string(),
            },
        ),
        MacOsControl::VolumeUp => (consumer_slot(0x00E9), Action::None),
        MacOsControl::VolumeDown => (consumer_slot(0x00EA), Action::None),
        MacOsControl::Mute => (consumer_slot(0x00E2), Action::None),
        MacOsControl::PlayPause => (consumer_slot(0x00CD), Action::None),
        MacOsControl::NextTrack => (consumer_slot(0x00B5), Action::None),
        MacOsControl::PreviousTrack => (consumer_slot(0x00B6), Action::None),
        MacOsControl::EmojiPicker => (keyboard_slot(0x09, 0x2C), Action::None),
    };
    input.emitted = emitted;
    input.action = action;
}

pub fn apply_app(input: &mut InputConfig, slot_index: usize, target: String) {
    input.behavior = Some(ControlBehavior::App {
        target: target.clone(),
    });
    input.emitted = host_trigger(slot_index);
    input.action = Action::Open { target };
}

pub fn behavior_is_consistent(input: &InputConfig, slot_index: usize) -> bool {
    let Some(behavior) = input.behavior.clone() else {
        return false;
    };
    let mut expected = input.clone();
    let valid = match behavior {
        ControlBehavior::ApplicationShortcut {
            application,
            shortcut,
        } => apply_application_shortcut(&mut expected, &application, &shortcut),
        ControlBehavior::MacOs { command }
            if matches!(command, MacOsControl::Applications | MacOsControl::Sleep) =>
        {
            apply_macos(&mut expected, slot_index, command);
            return hidden_trigger(input.emitted) && expected.action == input.action;
        }
        ControlBehavior::MacOs { command } => {
            apply_macos(&mut expected, slot_index, command);
            true
        }
        ControlBehavior::Keystroke => {
            return input.emitted.kind == SlotKind::Keyboard && input.action == Action::None;
        }
        ControlBehavior::App { target } => {
            apply_app(&mut expected, slot_index, target);
            return hidden_trigger(input.emitted) && expected.action == input.action;
        }
    };
    valid && expected.emitted == input.emitted && expected.action == input.action
}

/// Keep every host-assisted semantic behavior on a distinct, non-printing
/// chord. A user may freely assign an old hidden chord as a normal keystroke;
/// this allocator moves the hidden trigger instead of letting one press run
/// two behaviors.
pub fn normalize_hidden_triggers(profile: &mut Profile) {
    for slot_index in 0..profile.inputs.len() {
        if !is_host_assisted(&profile.inputs[slot_index]) {
            continue;
        }
        let current = profile.inputs[slot_index].emitted;
        let collision = profile
            .inputs
            .iter()
            .enumerate()
            .any(|(other, input)| other != slot_index && input.emitted == current);
        if hidden_trigger(current) && !collision {
            continue;
        }

        if let Some(candidate) = hidden_trigger_candidates().find(|candidate| {
            profile
                .inputs
                .iter()
                .enumerate()
                .all(|(other, input)| other == slot_index || input.emitted != *candidate)
        }) {
            profile.inputs[slot_index].emitted = candidate;
        }
    }
}

fn is_host_assisted(input: &InputConfig) -> bool {
    matches!(
        input.behavior.as_ref(),
        Some(ControlBehavior::App { .. })
            | Some(ControlBehavior::MacOs {
                command: MacOsControl::Applications | MacOsControl::Sleep
            })
    )
}

fn hidden_trigger(slot: Slot) -> bool {
    slot.kind == SlotKind::Keyboard && (0x68..=0x6F).contains(&slot.code) && slot.mods & 0xF0 == 0
}

fn hidden_trigger_candidates() -> impl Iterator<Item = Slot> {
    const MOD_BANKS: [u8; 16] = [
        0x00, 0x02, 0x03, 0x01, 0x04, 0x08, 0x06, 0x0A, 0x0C, 0x05, 0x09, 0x07, 0x0B, 0x0D, 0x0E,
        0x0F,
    ];
    MOD_BANKS
        .into_iter()
        .flat_map(|mods| (0x68..=0x6F).map(move |code| keyboard_slot(mods, code)))
}

fn keyboard_slot(mods: u8, key: u16) -> Slot {
    Slot {
        kind: SlotKind::Keyboard,
        mods,
        code: key,
    }
}

fn consumer_slot(code: u16) -> Slot {
    Slot {
        kind: SlotKind::Consumer,
        mods: 0,
        code,
    }
}

/// A unique, non-printing chord for host-assisted behavior. There are eight
/// F13–F20 keys in each modifier bank, enough to cover all 24 input slots.
fn host_trigger(slot_index: usize) -> Slot {
    const BANK_MODS: [u8; 3] = [0x00, 0x02, 0x03];
    keyboard_slot(
        BANK_MODS[(slot_index / 8).min(2)],
        0x68 + (slot_index % 8) as u16,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub path: String,
}

/// Discover launchable macOS app bundles from the normal user-facing roots.
/// Exact paths are persisted because two app bundles can share a bundle ID.
pub fn installed_apps() -> Vec<InstalledApp> {
    let mut paths = BTreeSet::new();
    for root in application_roots() {
        collect_app_bundles(&root, 0, 3, &mut paths);
    }

    let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        by_name.entry(stem.to_string()).or_default().push(path);
    }

    let mut apps = Vec::new();
    for (name, mut paths) in by_name {
        paths.sort();
        let duplicated = paths.len() > 1;
        for path in paths {
            let display_name = if duplicated {
                let parent = path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|value| value.to_str())
                    .unwrap_or("Applications");
                format!("{name} — {parent}")
            } else {
                name.clone()
            };
            apps.push(InstalledApp {
                name: display_name,
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    apps
}

fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

fn collect_app_bundles(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    found: &mut BTreeSet<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let is_app = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        if is_app {
            found.insert(path);
        } else if depth < max_depth {
            collect_app_bundles(&path, depth + 1, max_depth, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_codex_profile;
    use std::collections::HashSet;

    #[test]
    fn shortcut_catalog_is_well_formed() {
        let mut app_ids = HashSet::new();
        for app in APPLICATION_SHORTCUTS {
            assert!(app_ids.insert(app.id), "duplicate app id {}", app.id);
            assert!(!app.shortcuts.is_empty(), "{} has no shortcuts", app.id);
            let mut ids = HashSet::new();
            let mut chords = HashSet::new();
            for preset in app.shortcuts {
                assert!(
                    ids.insert(preset.id),
                    "{}: duplicate shortcut id {}",
                    app.id,
                    preset.id
                );
                assert!(
                    chords.insert((preset.mods, preset.key)),
                    "{}: duplicate chord on {}",
                    app.id,
                    preset.id
                );
                assert!(
                    keyboard_name(preset.key).is_some(),
                    "{}.{}: unknown key usage {:#04x}",
                    app.id,
                    preset.id,
                    preset.key
                );
                assert_eq!(
                    preset.mods & !0x0f,
                    0,
                    "{}.{}: chords use left-hand modifier bits only",
                    app.id,
                    preset.id
                );
                for label in preset.labels {
                    assert!(
                        !label.trim().is_empty(),
                        "{}.{}: empty label",
                        app.id,
                        preset.id
                    );
                    assert!(
                        label.chars().count() <= 34,
                        "{}.{}: label too long: {label}",
                        app.id,
                        preset.id
                    );
                }
            }
        }
    }

    #[test]
    fn shortcut_catalog_icons_resolve() {
        for app in APPLICATION_SHORTCUTS {
            if let Some(slug) = app.icon.strip_prefix("simple:") {
                assert!(
                    crate::simple_icons::find(slug).is_some(),
                    "{}: unknown Simple Icons slug {slug}",
                    app.id
                );
            } else {
                assert!(
                    crate::lucide::icon_char(app.icon).is_some(),
                    "{}: unknown Lucide icon {}",
                    app.id,
                    app.icon
                );
            }
        }
    }

    /// Saved profiles reference (application, shortcut) ids and re-derive the
    /// emitted chord from this catalog — these historical pairs must keep
    /// producing identical bytes forever.
    #[test]
    fn legacy_shortcut_ids_keep_their_chords() {
        #[rustfmt::skip]
        const FROZEN: &[(&str, &str, u8, u16)] = &[
            ("finder", "new_window", 0x08, 0x11), ("finder", "new_folder", 0x0A, 0x11),
            ("finder", "go_to_folder", 0x0A, 0x0A), ("finder", "get_info", 0x08, 0x0C),
            ("finder", "quick_look", 0x00, 0x2C), ("finder", "move_to_trash", 0x08, 0x2A),
            ("safari", "new_tab", 0x08, 0x17), ("safari", "close_tab", 0x08, 0x1A),
            ("safari", "reopen_tab", 0x0A, 0x17), ("safari", "address", 0x08, 0x0F),
            ("safari", "downloads", 0x0C, 0x0F),
            ("chrome", "new_tab", 0x08, 0x17), ("chrome", "close_tab", 0x08, 0x1A),
            ("chrome", "reopen_tab", 0x0A, 0x17), ("chrome", "address", 0x08, 0x0F),
            ("chrome", "incognito", 0x0A, 0x11),
            ("vscode", "command_palette", 0x0A, 0x13), ("vscode", "quick_open", 0x08, 0x13),
            ("vscode", "new_window", 0x0A, 0x11), ("vscode", "toggle_terminal", 0x01, 0x35),
            ("vscode", "find_in_files", 0x0A, 0x09),
            ("xcode", "build", 0x08, 0x05), ("xcode", "run", 0x08, 0x15),
            ("xcode", "test", 0x08, 0x18), ("xcode", "stop", 0x08, 0x37),
            ("xcode", "open_quickly", 0x0A, 0x12),
            ("terminal", "new_window", 0x08, 0x11), ("terminal", "new_tab", 0x08, 0x17),
            ("terminal", "clear", 0x08, 0x0E), ("terminal", "close", 0x08, 0x1A),
            ("terminal", "find", 0x08, 0x09),
            ("slack", "quick_switcher", 0x08, 0x0E), ("slack", "preferences", 0x08, 0x36),
            ("slack", "threads", 0x0A, 0x17), ("slack", "history_back", 0x08, 0x2F),
            ("slack", "history_forward", 0x08, 0x30),
            ("figma", "quick_actions", 0x08, 0x38), ("figma", "frame", 0x00, 0x09),
            ("figma", "pen", 0x00, 0x13), ("figma", "text", 0x00, 0x17),
            ("figma", "components", 0x0C, 0x0E),
            ("zoom", "mute", 0x0A, 0x04), ("zoom", "video", 0x0A, 0x19),
            ("zoom", "share", 0x0A, 0x16), ("zoom", "participants", 0x08, 0x18),
            ("zoom", "chat", 0x0A, 0x0B),
        ];
        for (app, id, mods, key) in FROZEN {
            let preset = shortcut_preset(app, id)
                .unwrap_or_else(|| panic!("{app}.{id} vanished from the catalog"));
            assert_eq!(
                (preset.mods, preset.key),
                (*mods, *key),
                "{app}.{id} chord changed"
            );
        }
    }

    fn sample_input() -> InputConfig {
        InputConfig {
            label: "Key".to_string(),
            icon: "star".to_string(),
            behavior: None,
            emitted: Slot::default(),
            action: Action::None,
        }
    }

    #[test]
    fn application_shortcut_compiles_to_one_saved_chord() {
        let mut input = sample_input();
        assert!(apply_application_shortcut(
            &mut input,
            "vscode",
            "command_palette"
        ));
        assert_eq!(
            input.emitted,
            Slot {
                kind: SlotKind::Keyboard,
                mods: 0x0A,
                code: 0x13,
            }
        );
        assert_eq!(input.action, Action::None);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::ApplicationShortcut {
                application: "vscode".to_string(),
                shortcut: "command_palette".to_string(),
            })
        );
    }

    #[test]
    fn host_app_behavior_gets_a_unique_non_printing_trigger() {
        let mut first = sample_input();
        let mut touch = sample_input();
        apply_app(&mut first, 0, "/Applications/Finder.app".to_string());
        apply_app(&mut touch, 21, "/Applications/Music.app".to_string());
        assert_ne!(first.emitted, touch.emitted);
        assert_eq!(first.emitted.kind, SlotKind::Keyboard);
        assert_eq!(touch.emitted.kind, SlotKind::Keyboard);
    }

    #[test]
    fn macos_sleep_keeps_semantics_above_its_execution_mapping() {
        let mut input = sample_input();
        apply_macos(&mut input, 4, MacOsControl::Sleep);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::Sleep
            })
        );
        assert!(matches!(input.action, Action::Run { .. }));
    }

    #[test]
    fn macos_globe_uses_apples_native_consumer_usage() {
        let mut input = sample_input();
        apply_macos(&mut input, 4, MacOsControl::Globe);
        assert_eq!(
            input.emitted,
            Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0x029D,
            }
        );
        assert_eq!(input.action, Action::None);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::Globe
            })
        );
        assert!(behavior_is_consistent(&input, 4));
    }

    #[test]
    fn direct_chord_collision_reallocates_only_the_hidden_trigger() {
        let mut profile = default_codex_profile();
        apply_app(
            &mut profile.inputs[0],
            0,
            "/Applications/Finder.app".to_string(),
        );
        let hidden_before = profile.inputs[0].emitted;
        apply_keystroke(
            &mut profile.inputs[1],
            hidden_before.mods,
            hidden_before.code,
        );
        let direct = profile.inputs[1].emitted;

        normalize_hidden_triggers(&mut profile);

        assert_eq!(profile.inputs[1].emitted, direct);
        assert_ne!(profile.inputs[0].emitted, direct);
        assert!(behavior_is_consistent(&profile.inputs[0], 0));
        assert!(behavior_is_consistent(&profile.inputs[1], 1));
    }
}
