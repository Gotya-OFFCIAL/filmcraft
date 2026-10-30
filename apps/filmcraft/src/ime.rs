//! The Android IME bridge: soft-keyboard text into egui.
//!
//! winit's Android back end shows the keyboard on `Window::set_ime_allowed` but never
//! delivers the typed text (it emits no `Ime` events), and NativeActivity's content view is
//! not a text editor, so `showSoftInput` has nothing to attach to. The shell therefore keeps
//! a hidden `EditText` (`packaging/android/java/ai/storyteller/filmcraft/ImeBridge.java`,
//! compiled to classes.dex by `packaging/android/build-apk.sh`): a real editor that owns the
//! IME's `InputConnection` and mirrors every edit here over JNI.
//!
//! State model: the bridge reports the full text and selection after each edit (the same
//! contract as android-activity's `TextInputState`), and [`pump`] turns the difference from
//! the last acknowledged state into `egui` input events for the focused text widget:
//! appends become [`egui::Event::Text`], deletions at the end become Backspace keys, and any
//! other edit is applied as select-all + replace. [`pump`] also shows the keyboard when an
//! egui text widget asks for IME and hides it when the widget lets go.
//!
//! `unsafe` is confined to the JNI boundary (ADR 0001 pattern: deny in the crate, allow per
//! item with a `SAFETY:` reason). The diff logic below is platform-independent and tested.

use eframe::egui;

/// The edits to apply to the focused egui widget to turn `old` into `new`.
fn diff_events(old: &str, new: &str) -> Vec<egui::Event> {
    if old == new {
        return Vec::new();
    }
    if let Some(appended) = new.strip_prefix(old) {
        return vec![egui::Event::Text(appended.to_string())];
    }
    if new.len() < old.len() && old.starts_with(new) {
        return vec![
            egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            old.len() - new.len()
        ];
    }
    // Any other edit: select all and replace, which every egui text widget handles.
    vec![
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { ctrl: true, ..egui::Modifiers::NONE },
        },
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers { ctrl: true, ..egui::Modifiers::NONE },
        },
        egui::Event::Text(new.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::diff_events;
    use eframe::egui;

    fn text_of(events: &[egui::Event]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn append_becomes_text() {
        assert_eq!(text_of(&diff_events("Main", "Main Edit")), " Edit");
    }

    #[test]
    fn truncation_becomes_backspaces() {
        let ev = diff_events("Main Edit", "Main");
        assert!(ev.len() == 5 && ev.iter().all(|e| matches!(e, egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. })));
    }

    #[test]
    fn any_other_edit_replaces() {
        let ev = diff_events("Main Edit", "Main.mp4");
        assert!(matches!(ev.as_slice(), [egui::Event::Key { .. }, egui::Event::Key { .. }, egui::Event::Text(t)] if t == "Main.mp4"));
    }

    #[test]
    fn same_text_is_silent() {
        assert!(diff_events("abc", "abc").is_empty());
    }
}

// ------------------------------------------------------------------ Android-only bridge

#[cfg(target_os = "android")]
mod bridge {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use eframe::egui;

    /// What the Java bridge reported since the last `take`.
    #[derive(Default)]
    pub struct Report {
        pub text: Option<String>,
        pub selection: Option<(usize, usize)>,
        pub action: Option<i32>,
    }

    /// Full state the bridge acknowledged last (the baseline for the next diff).
    #[derive(Default)]
    struct Acked {
        text: String,
    }

    static REPORT: Mutex<Report> = Mutex::new(Report { text: None, selection: None, action: None });
    static ACKED: Mutex<Acked> = Mutex::new(Acked { text: String::new() });
    /// JNI handles captured at [`install`] (global refs; the class loader of an attached
    /// thread cannot see app classes, so every later call goes through these).
    static JNI: Mutex<Option<Jni>> = Mutex::new(None);
    /// The egui IME-focus state last frame, for show/hide edges.
    static IME_WANTED: Mutex<bool> = Mutex::new(false);

    struct Jni {
        vm: jni::JavaVM,
        class: jni::objects::GlobalRef,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called from Java (`ImeBridge.onTextChanged`) after each edit of the hidden editor.
    #[allow(unsafe_code)]
    // SAFETY: exported for the JVM; registered via `register_native_methods` against the
    // signature `(Ljava/lang/String;II)V`, so the arguments match, and the env is the one
    // the VM passes for this call. Only extracts copies of the strings.
    unsafe extern "system" fn text_changed(
        mut env: jni::JNIEnv,
        _class: jni::objects::JClass,
        text: jni::objects::JString,
        sel_start: jni::sys::jint,
        sel_end: jni::sys::jint,
    ) {
        let s: String = env.get_string(&text).map(Into::into).unwrap_or_default();
        let mut r = lock(&REPORT);
        r.text = Some(s);
        r.selection = Some((sel_start.max(0) as usize, sel_end.max(0) as usize));
    }

    /// Called from Java (`ImeBridge.onAction`) when the IME action key (Done, Next, ...) fires.
    #[allow(unsafe_code)]
    // SAFETY: exported for the JVM; registered against `(I)V`. Only copies an integer.
    unsafe extern "system" fn action(_env: jni::JNIEnv, _class: jni::objects::JClass, action: jni::sys::jint) {
        lock(&REPORT).action = Some(action);
    }

    /// Attach the hidden editor to the activity and register the JNI callbacks. Idempotent.
    #[allow(unsafe_code)]
    // SAFETY: the raw VM/activity pointers come from AndroidApp (live for the activity's
    // lifetime; this runs during android_main). JObject has no Drop, so wrapping the
    // activity reference for these calls does not delete it. Every JNI call is checked.
    pub fn install(app: &winit::platform::android::activity::AndroidApp) -> Result<(), String> {
        use jni::objects::{JObject, JValue};
        if lock(&JNI).is_some() {
            return Ok(());
        }
        let vm = app.vm_as_ptr() as *mut jni::sys::JavaVM;
        let activity = app.activity_as_ptr() as jni::sys::jobject;
        unsafe {
            let vm = jni::JavaVM::from_raw(vm).map_err(|e| format!("JavaVM: {e}"))?;
            // The env (AttachGuard) borrows the vm, so every env call ends in this inner
            // scope; only then may the vm move into the JNI static below.
            let class = {
                let mut env = vm.attach_current_thread().map_err(|e| format!("attach: {e}"))?;
                let activity = JObject::from_raw(activity);
                // An attached native thread's find_class sees the boot loader only; ask the
                // activity's own loader for ImeBridge instead.
                let loader = env
                    .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
                    .and_then(|v| v.l())
                    .map_err(|e| format!("getClassLoader: {e}"))?;
                let name = env.new_string("ai.storyteller.filmcraft.ImeBridge").map_err(|e| format!("new_string: {e}"))?;
                let class = env
                    .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])
                    .and_then(|v| v.l())
                    .map_err(|e| format!("loadClass: {e}"))?;
                let class = jni::objects::JClass::from(class);
                // The library was dlopened under NativeActivity's class loader, so name-based
                // lookup would not bind these natives to ImeBridge: register them explicitly.
                let methods = [
                    jni::NativeMethod { name: "onTextChanged".into(), sig: "(Ljava/lang/String;II)V".into(), fn_ptr: text_changed as *mut _ },
                    jni::NativeMethod { name: "onAction".into(), sig: "(I)V".into(), fn_ptr: action as *mut _ },
                ];
                env.register_native_methods(&class, &methods).map_err(|e| format!("register: {e}"))?;
                env.call_static_method(&class, "install", "(Landroid/app/NativeActivity;)V", &[JValue::Object(&activity)])
                    .map_err(|e| format!("ImeBridge.install: {e}"))?;
                env.new_global_ref(&class).map_err(|e| format!("global ref: {e}"))?
            };
            *lock(&JNI) = Some(Jni { vm, class });
        }
        Ok(())
    }

    /// Show the keyboard for the hidden editor, seeded with `text`/selection.
    pub fn show(text: &str, selection: (usize, usize)) {
        call_editor("show", text, selection);
    }

    /// Mirror application-side edits into the hidden editor (no echo back).
    #[allow(dead_code)]
    pub fn update(text: &str, selection: (usize, usize)) {
        call_editor("update", text, selection);
    }

    /// Hide the keyboard and drop focus.
    pub fn hide() {
        let guard = lock(&JNI);
        let Some(jni) = guard.as_ref() else { return };
        if let Err(e) = jni.vm.attach_current_thread().and_then(|mut env| {
            env.call_static_method(&jni.class, "hide", "()V", &[])?;
            Ok(())
        }) {
            log::warn!("ime hide: {e}");
        }
    }

    fn call_editor(method: &str, text: &str, selection: (usize, usize)) {
        use jni::objects::JValue;
        let guard = lock(&JNI);
        let Some(jni) = guard.as_ref() else { return };
        let res = jni.vm.attach_current_thread().and_then(|mut env| {
            let t = env.new_string(text)?;
            env.call_static_method(
                &jni.class,
                method,
                "(Ljava/lang/String;II)V",
                &[JValue::Object(&t), JValue::Int(selection.0 as i32), JValue::Int(selection.1 as i32)],
            )?;
            Ok(())
        });
        if let Err(e) = res {
            log::warn!("ime {method}: {e}");
        }
    }

    /// Per frame: deliver bridge edits to the focused egui widget and follow its IME focus.
    pub fn pump(ctx: &egui::Context) {
        let report = std::mem::take(&mut *lock(&REPORT));
        if let Some(text) = report.text {
            let mut acked = lock(&ACKED);
            let events = super::diff_events(&acked.text, &text);
            acked.text = text.clone();
            drop(acked);
            if !events.is_empty() {
                ctx.input_mut(|i| i.events.extend(events));
            }
            log::info!("ime: text len {} sel {:?}", text.len(), report.selection);
            if let Some(action) = report.action {
                log::info!("ime: action {action}");
            }
        } else if let Some(action) = report.action {
            log::info!("ime: action {action}");
        }
        // Follow the egui text focus. `platform_output.ime` is not usable here: it is built
        // during the pass (after this callback) and read back empty at the start of the next
        // one. The focus flag lives in Memory and answers correctly in on_begin_pass.
        let wants = ctx.text_edit_focused();
        let mut wanted = lock(&IME_WANTED);
        if wants != *wanted {
            *wanted = wants;
            log::info!("ime: text focus {wants}");
            if wants {
                let text = lock(&ACKED).text.clone();
                show(&text, (text.len(), text.len()));
            } else {
                hide();
            }
        }
    }
}

#[cfg(target_os = "android")]
pub use bridge::{install, pump};
