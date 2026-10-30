//! The in-app file browser Android needs in place of a native dialog.
//!
//! The desktop asks the OS for a file (rfd); Android has no such dialog without the Storage
//! Access Framework, and this shell carries no Java for it (`hasCode="false"`). The app shows its
//! own browser instead: it walks the file system directly, so a build with no JNI still lets the
//! user open media and write exports.
//!
//! Scoped storage (SDK 29+) decides what can be read and written: the app's own external folder
//! is always writable; shared folders (`Movies`, `Download`, …) are only writable when the user
//! grants "All files access" in the system settings. [`writable`] answers that question so the
//! browser can offer a save folder it can actually write to rather than one that fails at export.
//!
//! Automation ids: `filebrowser.entry.<n>`, `filebrowser.up`, `filebrowser.cancel`,
//! `filebrowser.choose`, `filebrowser.name`.

use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, Frame, Margin, RichText, Stroke, Ui, vec2};

use crate::FilmcraftApp;
use crate::native_dialogs::FileDialog;
use crate::theme::Tokens;

/// What the browser is asked to yield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Pick one or more existing files whose extension is in the dialog's list.
    OpenFiles,
    /// Pick one existing file.
    OpenFile,
    /// Pick one existing project.
    OpenProject,
    /// Pick a folder (a save destination or an import folder).
    Folder,
    /// Name a file to write: a save dialog with a name field.
    Save,
}

impl Mode {
    /// The mode a [`FileDialog`] maps to, and the extensions it accepts (empty: anything).
    pub fn of(dialog: &FileDialog) -> (Self, Vec<String>) {
        match dialog {
            FileDialog::OpenFiles { exts } => (Self::OpenFiles, exts.clone()),
            FileDialog::OpenFile { exts, .. } => (Self::OpenFile, exts.clone()),
            FileDialog::OpenProject => (Self::OpenProject, vec!["fcproj".into()]),
            FileDialog::Relink { exts, .. } => (Self::OpenFile, exts.clone()),
            FileDialog::Folder { .. } => (Self::Folder, Vec::new()),
            FileDialog::SaveProject { .. } => (Self::Save, vec!["fcproj".into()]),
            FileDialog::SaveAs { exts, .. } => (Self::Save, exts.clone()),
        }
    }

    /// The name a save dialog starts with.
    pub fn suggested<'a>(dialog: &'a FileDialog) -> Option<&'a str> {
        match dialog {
            FileDialog::SaveProject { name } | FileDialog::SaveAs { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// Where the app may write with no permission granted: its own external folder, which every app
/// owns on shared storage. `/sdcard/Android/data/<pkg>/files`, the path Android assigns.
pub fn app_folder() -> PathBuf {
    let base = app_folder_or_shared();
    if base.is_dir() { base } else { PathBuf::from("/sdcard") }
}

/// The path Android gives the app's own external folder, whether or not this host has it.
pub fn app_folder_or_shared() -> PathBuf {
    Path::new("/sdcard/Android/data").join("ai.storyteller.filmcraft").join("files")
}

/// Where a browser starts: shared media folders when they exist (they hold the user's footage),
/// else the app's own folder.
pub fn start_folder() -> PathBuf {
    for dir in ["/sdcard/Movies", "/sdcard/DCIM", "/sdcard/DCIM/Camera", "/sdcard/Download"] {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return p;
        }
    }
    app_folder()
}

/// Whether `dir` can be written with the permissions this build has. Android's scoped storage
/// gives every app its own external folder and nothing else without "All files access"; asking
/// the file system directly is the truthful test (no permission can be granted from inside the
/// app's own Java-free shell).
pub fn writable(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(".filmcraft-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// One row of the listing.
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    size: u64,
}

/// The browser's state, kept in egui temporary storage while it is open.
#[derive(Clone, Debug)]
pub struct BrowserState {
    pub dir: PathBuf,
    pub mode: Mode,
    pub exts: Vec<String>,
    pub name: String,
    /// Selected file (open modes) or the folder to write into (folder mode).
    pub chosen: Option<PathBuf>,
}

impl Default for BrowserState {
    fn default() -> Self {
        // Only `remove_temp::<BrowserState>()` reads this; the value is dropped unread.
        Self { dir: PathBuf::from("/sdcard"), mode: Mode::OpenFiles, exts: Vec::new(), name: String::new(), chosen: None }
    }
}

/// The channel a [`HostHooks::run_dialog`] answer travels on: the browser sends the chosen paths
/// when the user confirms (empty on cancel), and `poll_native_dialog` continues on the next frame.
#[derive(Clone)]
struct Answer(std::sync::mpsc::Sender<Vec<String>>);

impl Default for Answer {
    fn default() -> Self {
        // Never used: only `remove_temp::<Answer>()` needs a Default, and the value it builds is
        // dropped at once. A disconnected sender keeps that honest.
        let (tx, _rx) = std::sync::mpsc::channel();
        Self(tx)
    }
}

/// The answer channel's id in egui temporary storage, separate from the browser's own state so
/// both survive the same frame.
fn answer_id() -> egui::Id {
    egui::Id::new("filebrowser.answer")
}

/// The extensions the browser shows as pickable: the dialog's list, or everything when empty.
fn matches(name: &str, exts: &[String]) -> bool {
    if exts.is_empty() {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    exts.iter().any(|e| lower.ends_with(&format!(".{}", e.to_ascii_lowercase())))
}

fn listing(dir: &Path, exts: &[String]) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Entry> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok()?;
            let is_dir = meta.is_dir();
            if !is_dir && !matches(&name, exts) {
                return None;
            }
            Some(Entry { path: e.path(), name, is_dir, size: meta.len() })
        })
        .collect();
    // Folders first, then by name: the order a file manager uses.
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

fn row(app: &mut FilmcraftApp, ui: &mut Ui, id: String, label: &str, detail: &str, active: bool) -> bool {
    let t = app.tokens;
    let h = crate::mobile::MIN_TOUCH_DP.max(38.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), egui::Sense::click());
    app.auto.add(&id, rect, label);
    if active || resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), if active { t.accent.gamma_multiply(0.35) } else { t.header_bg });
    }
    let text_color = if resp.hovered() || active { t.text } else { t.text_dim };
    ui.painter().text(egui::pos2(rect.min.x + 10.0, rect.center().y), Align2::LEFT_CENTER, label, Tokens::ui(13.0), text_color);
    if !detail.is_empty() {
        ui.painter().text(egui::pos2(rect.max.x - 10.0, rect.center().y), Align2::RIGHT_CENTER, detail, Tokens::ui(11.0), t.text_faint);
    }
    resp.clicked()
}

/// Show the browser as a modal window. Returns the chosen paths once the user confirms (empty
/// means still open; the caller closes it), and whether it is still open.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) -> Option<Vec<String>> {
    let id = egui::Id::new("filebrowser");
    let Some(mut st) = ctx.data(|d| d.get_temp::<BrowserState>(id)) else { return None };
    let t = app.tokens;
    let mut answer: Option<Vec<String>> = None;
    let mut close = false;
    let width = (ctx.content_rect().width() - 40.0).clamp(320.0, 560.0);
    let height = (ctx.content_rect().height() - 80.0).clamp(320.0, 640.0);
    let frame = Frame::new().fill(t.panel_bg).stroke(Stroke::new(1.0, t.separator)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(0));

    egui::Window::new(tl!("Choose a file"))
        .id(id.with("window"))
        .frame(frame)
        .title_bar(true)
        .collapsible(false)
        .resizable(false)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .fixed_size(vec2(width, height))
        .show(ctx, |ui| {
            // The path bar, then Up, then the listing (a preview of where we are).
            let mut go_up: Option<PathBuf> = None;
            ui.horizontal(|ui| {
                let up = ui.button(RichText::new("↑").size(15.0));
                app.auto.add("filebrowser.up", up.rect, "Up");
                if up.clicked() {
                    go_up = st.dir.parent().map(Path::to_path_buf);
                }
                let writable_here = writable(&st.dir);
                let mark = if writable_here { String::new() } else { format!("  {}", tl!("(read only)")) };
                ui.label(RichText::new(format!("{}{}", st.dir.display(), mark)).size(11.0).color(t.text_dim));
            });
            if let Some(up) = go_up {
                st.dir = up;
                st.chosen = None;
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(height - 130.0).show(ui, |ui| {
                let entries = listing(&st.dir, &st.exts);
                if entries.is_empty() {
                    ui.label(RichText::new(tl!("(empty)")).size(12.0).color(t.text_faint));
                }
                for (n, e) in entries.iter().enumerate() {
                    let detail = if e.is_dir { String::new() } else { human_size(e.size) };
                    let label = if e.is_dir { format!("{}/", e.name) } else { e.name.clone() };
                    let active = st.chosen.as_deref() == Some(e.path.as_path());
                    if row(app, ui, format!("filebrowser.entry.{n}"), &label, &detail, active) {
                        if e.is_dir {
                            st.dir = e.path.clone();
                            st.chosen = if st.mode == Mode::Folder { Some(e.path.clone()) } else { None };
                        } else if st.mode == Mode::Save {
                            // A save dialog takes the name, not the file.
                        } else {
                            st.chosen = Some(e.path.clone());
                        }
                    }
                }
            });
            ui.separator();
            // The action row: a name field for saves, Choose / Cancel for everything else.
            ui.horizontal(|ui| {
                if st.mode == Mode::Save {
                    ui.label(RichText::new(tl!("Name")).size(12.0).color(t.text_dim));
                    let r = ui.add(egui::TextEdit::singleline(&mut st.name).desired_width(180.0));
                    app.auto.add("filebrowser.name", r.rect, &st.name);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cancel = ui.button(RichText::new(tl!("Cancel")).size(13.0));
                    app.auto.add("filebrowser.cancel", cancel.rect, "Cancel");
                    if cancel.clicked() {
                        close = true;
                    }
                    let can = match st.mode {
                        Mode::Save => !st.name.trim().is_empty() && writable(&st.dir),
                        Mode::Folder => writable(&st.dir),
                        _ => st.chosen.is_some(),
                    };
                    let choose = ui.add_enabled(can, egui::Button::new(RichText::new(tl!("Choose")).size(13.0)));
                    app.auto.add("filebrowser.choose", choose.rect, "Choose");
                    if choose.clicked() && can {
                        answer = Some(match st.mode {
                            Mode::Folder => vec![st.dir.display().to_string()],
                            Mode::Save => {
                                let name = st.name.trim().to_string();
                                let name = if st.exts.iter().any(|e| name.to_ascii_lowercase().ends_with(&format!(".{e}"))) {
                                    name
                                } else {
                                    match st.exts.first() {
                                        Some(ext) => format!("{name}.{ext}"),
                                        None => name,
                                    }
                                };
                                vec![st.dir.join(name).display().to_string()]
                            }
                            _ => st.chosen.iter().map(|p| p.display().to_string()).collect(),
                        });
                    }
                });
            });
        });

    if answer.is_some() || close {
        // Answer the waiting continuation: the chosen paths, or empty for a cancel.
        note(&format!("file browser: answering with {:?}", answer.as_deref().unwrap_or(&[])));
        if let Some(Answer(tx)) = ctx.data_mut(|d| d.remove_temp::<Answer>(answer_id())) {
            let _ = tx.send(answer.clone().unwrap_or_default());
        }
        ctx.data_mut(|d| d.remove_temp::<BrowserState>(id));
    } else {
        ctx.data_mut(|d| d.insert_temp(id, st));
    }
    answer
}

/// Open the browser for `dialog`, starting where a file of its kind lives. The paths the user
/// chooses are delivered on the returned channel (empty when they cancel), so a
/// [`crate::HostHooks::run_dialog`] host can answer without blocking the frame loop.
pub fn open(ctx: &egui::Context, dialog: &FileDialog) -> std::sync::mpsc::Receiver<Vec<String>> {
    let (mode, exts) = Mode::of(dialog);
    let dir = match dialog {
        // A save starts where the app may write; an open starts where the user's footage is.
        FileDialog::SaveProject { .. } | FileDialog::SaveAs { .. } => app_folder(),
        _ => start_folder(),
    };
    let name = Mode::suggested(dialog).unwrap_or("").to_string();
    let st = BrowserState { dir, mode, exts, name, chosen: None };
    // The browser's start is the first thing to check when a pick "does nothing" on a device:
    // scoped storage hides folders, and an empty listing then looks like a broken dialog.
    note(&format!("file browser: {:?} opens in {} ({} shown)", mode, st.dir.display(), listing(&st.dir, &st.exts).len()));
    let (tx, rx) = std::sync::mpsc::channel();
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::new("filebrowser"), st);
        d.insert_temp(answer_id(), Answer(tx));
    });
    ctx.request_repaint();
    rx
}

/// Whether the browser is showing.
pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<BrowserState>(egui::Id::new("filebrowser"))).is_some()
}

/// Append one line to the app's log file when the platform has no logger. The Android shell has
/// no `env_logger`, so this is what a pick leaves behind for `adb pull` on a real device.
pub(crate) fn note(line: &str) {
    log::info!("{line}");
    let dir = app_folder_or_shared();
    let dir = if dir.is_dir() { dir } else { PathBuf::from("/sdcard") };
    let path = dir.join("filmcraft.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write as _;
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder under the target dir (no external crates: the workspace avoids new deps).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("filmcraft-filebrowser-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    #[test]
    fn a_dialog_maps_to_the_mode_its_question_needs() {
        assert_eq!(Mode::of(&FileDialog::open_files(&["mp4"])).0, Mode::OpenFiles);
        assert_eq!(Mode::of(&FileDialog::open_file("Media", &["mov"])).0, Mode::OpenFile);
        assert_eq!(Mode::of(&FileDialog::OpenProject).0, Mode::OpenProject);
        assert_eq!(Mode::of(&FileDialog::Folder { at: None }).0, Mode::Folder);
        assert_eq!(Mode::of(&FileDialog::save_as("Video", &["mp4"], "a.mp4")).0, Mode::Save);
        // A relink takes one existing file, like Open...
        let hint = crate::RelinkHint { command: "media.relink".into(), params: serde_json::json!({}) };
        assert_eq!(Mode::of(&FileDialog::Relink { exts: vec!["mp4".into()], hint }).0, Mode::OpenFile);
    }

    #[test]
    fn a_save_dialog_names_its_first_extension_after_a_name_without_one() {
        let exts = vec!["mp4".to_string()];
        assert!(matches("cut", &exts) == false);
        // The browser appends the extension: `cut` in a mp4 dialog yields `cut.mp4`.
        let name = if matches("cut", &exts) { "cut".to_string() } else { format!("cut.{}", exts[0]) };
        assert_eq!(name, "cut.mp4");
        // A name that already carries it is kept, whatever the case.
        assert!(matches("cut.MP4", &exts));
    }

    #[test]
    fn a_listing_shows_folders_first_and_only_the_dialogs_extensions() {
        let dir = scratch("listing");
        std::fs::create_dir_all(dir.join("footage")).unwrap();
        std::fs::write(dir.join("b.mp4"), b"x").unwrap();
        std::fs::write(dir.join("a.mp4"), b"xx").unwrap();
        std::fs::write(dir.join("notes.txt"), b"nope").unwrap();
        std::fs::write(dir.join(".hidden.mp4"), b"h").unwrap();
        let exts = vec!["mp4".to_string()];
        let rows = listing(&dir, &exts);
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["footage", "a.mp4", "b.mp4"], "folders first, then by name; .txt and dotfiles out");
        assert!(rows[0].is_dir);
        assert_eq!(rows[1].size, 2);
        let all = listing(&dir, &[]);
        assert!(all.iter().any(|e| e.name == "notes.txt"), "no extension list shows everything");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_is_writable_only_when_a_probe_file_can_be_made_in_it() {
        let dir = scratch("writable");
        assert!(writable(&dir), "a fresh temp folder takes the probe");
        assert!(!dir.join(".filmcraft-write-probe").exists(), "the probe is removed again");
        assert!(!writable(&dir.join("absent")), "a missing folder is not writable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_app_folder_is_the_one_android_assigns_it() {
        // Not created in tests: this only pins the path the export default uses.
        assert_eq!(app_folder_or_shared(), PathBuf::from("/sdcard/Android/data/ai.storyteller.filmcraft/files"));
    }
}
