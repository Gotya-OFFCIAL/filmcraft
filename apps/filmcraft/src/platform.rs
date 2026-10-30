//! Platform picks and shares (Android): the system document picker and the share/player
//! Intents behind the export-complete panel.
//!
//! Import asks Android's own Storage Access Framework picker (`Intent.ACTION_OPEN_DOCUMENT`)
//! instead of the in-app browser — the platform picker is the Android-native way to browse
//! documents, and every provider (Drive, NAS plugins, Downloads) appears in it. The picked
//! `content://` documents are copied into the app cache by `PlatformBridge.java` and answered
//! as real paths, which is the contract `FileDialog` already has. Save and folder picks stay
//! with the in-app browser (`panels::file_browser`): they target paths the app itself writes.
//!
//! Preview and share hand a finished export to the system: the Java side scans the file into
//! the MediaStore (an `ACTION_VIEW`/`ACTION_SEND` needs a content:// uri on API 24+, and a
//! bare file:// one throws) and starts the player or the share sheet.
//!
//! JNI notes match `src/ime.rs`: natives are registered explicitly (the library is dlopened
//! under NativeActivity's class loader, name lookup would not bind them), classes come from
//! the activity's own loader, and `unsafe` stays at the JNI boundary with `SAFETY:` reasons.

/// MIME types for a picker narrowed by file extension; unknown extensions widen to `*/*`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn mimes_for(exts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in exts {
        let mime = match e.to_ascii_lowercase().as_str() {
            "mp4" | "m4v" => "video/mp4",
            "mov" => "video/quicktime",
            "webm" => "video/webm",
            "mkv" => "video/x-matroska",
            "avi" => "video/x-msvideo",
            "wav" => "audio/wav",
            "mp3" => "audio/mpeg",
            "aac" | "m4a" => "audio/mp4",
            "flac" => "audio/flac",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "tif" | "tiff" => "image/tiff",
            "webp" => "image/webp",
            "exr" => "image/x-exr",
            "dpx" => "image/x-dpx",
            "gif" => "image/gif",
            _ => continue,
        };
        if !out.iter().any(|x| x == mime) {
            out.push(mime.to_string());
        }
    }
    if out.is_empty() {
        out.push("*/*".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::mimes_for;

    #[test]
    fn media_extensions_map_to_categories() {
        let m = mimes_for(&["mp4".into(), "mov".into(), "wav".into(), "png".into()]);
        assert_eq!(m, ["video/mp4", "video/quicktime", "audio/wav", "image/png"]);
    }

    #[test]
    fn unknown_extension_widens() {
        assert_eq!(mimes_for(&["fcproj".into()]), ["*/*"]);
    }
}

#[cfg(target_os = "android")]
mod bridge {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use jni::objects::{JClass, JObject, JObjectArray, JString, JValue};

    use super::mimes_for;

    /// Pending pick requests: token → the channel the dialog answer travels on.
    static PENDING: Mutex<Option<HashMap<u32, Sender<Vec<String>>>>> = Mutex::new(None);
    static NEXT_TOKEN: AtomicU32 = AtomicU32::new(1);

    struct Jni {
        vm: jni::JavaVM,
        class: jni::objects::GlobalRef,
    }

    static JNI: Mutex<Option<Jni>> = Mutex::new(None);

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called from Java (`PlatformBridge.onPicked`) with cache copies of the picked documents.
    #[allow(unsafe_code)]
    // SAFETY: exported for the JVM; registered against `([Ljava/lang/String;I)V`, so the
    // arguments match. Only copies strings out of the array.
    unsafe extern "system" fn picked(mut env: jni::JNIEnv, _class: JClass, paths: JObjectArray, token: jni::sys::jint) {
        let mut out = Vec::new();
        let n = env.get_array_length(&paths).unwrap_or(0);
        for i in 0..n {
            if let Ok(obj) = env.get_object_array_element(&paths, i)
                && let Ok(s) = env.get_string(&JString::from(obj))
            {
                out.push(String::from(s));
            }
        }
        answer(token as u32, out);
    }

    /// Called from Java (`PlatformBridge.onPickCancelled`) when the picker is dismissed.
    #[allow(unsafe_code)]
    // SAFETY: exported for the JVM; registered against `(I)V`. Only copies an integer.
    unsafe extern "system" fn pick_cancelled(_env: jni::JNIEnv, _class: JClass, token: jni::sys::jint) {
        answer(token as u32, Vec::new());
    }

    fn answer(token: u32, paths: Vec<String>) {
        let sender = lock(&PENDING).as_mut().and_then(|m| m.remove(&token));
        if let Some(tx) = sender {
            let _ = tx.send(paths);
        }
    }

    /// Attach PlatformBridge and register the JNI callbacks. Idempotent.
    #[allow(unsafe_code)]
    // SAFETY: the raw VM/activity pointers come from AndroidApp (live for the activity's
    // lifetime). JObject has no Drop, so wrapping the activity reference for these calls does
    // not delete it. Every JNI call is checked (same contract as ime.rs::install).
    pub fn install(app: &winit::platform::android::activity::AndroidApp) -> Result<(), String> {
        if lock(&JNI).is_some() {
            return Ok(());
        }
        let vm = app.vm_as_ptr() as *mut jni::sys::JavaVM;
        let activity = app.activity_as_ptr() as jni::sys::jobject;
        unsafe {
            let vm = jni::JavaVM::from_raw(vm).map_err(|e| format!("JavaVM: {e}"))?;
            let class = {
                let mut env = vm.attach_current_thread().map_err(|e| format!("attach: {e}"))?;
                let activity = JObject::from_raw(activity);
                let loader = env
                    .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
                    .and_then(|v| v.l())
                    .map_err(|e| format!("getClassLoader: {e}"))?;
                let name = env.new_string("ai.storyteller.filmcraft.PlatformBridge").map_err(|e| format!("new_string: {e}"))?;
                let class = env
                    .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])
                    .and_then(|v| v.l())
                    .map_err(|e| format!("loadClass: {e}"))?;
                let class = JClass::from(class);
                let methods = [
                    jni::NativeMethod { name: "onPicked".into(), sig: "([Ljava/lang/String;I)V".into(), fn_ptr: picked as *mut _ },
                    jni::NativeMethod { name: "onPickCancelled".into(), sig: "(I)V".into(), fn_ptr: pick_cancelled as *mut _ },
                ];
                env.register_native_methods(&class, &methods).map_err(|e| format!("register: {e}"))?;
                env.call_static_method(&class, "install", "(Landroid/app/NativeActivity;)V", &[JValue::Object(&activity)])
                    .map_err(|e| format!("PlatformBridge.install: {e}"))?;
                env.new_global_ref(&class).map_err(|e| format!("global ref: {e}"))?
            };
            *lock(&JNI) = Some(Jni { vm, class });
        }
        Ok(())
    }

    /// Open the system document picker; the answer arrives on the returned channel (empty =
    /// cancelled). `multiple` allows multi-select; `exts` narrows the picker by MIME type.
    pub fn pick_files(multiple: bool, exts: &[String]) -> Receiver<Vec<String>> {
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        lock(&PENDING).get_or_insert_with(HashMap::new).insert(token, tx);
        let mimes = mimes_for(exts);
        if let Err(e) = pick_files_jni(multiple, &mimes, token) {
            log::warn!("platform pick_files: {e}");
            answer(token, Vec::new());
        }
        rx
    }

    fn pick_files_jni(multiple: bool, mimes: &[String], token: u32) -> Result<(), String> {
        let guard = lock(&JNI);
        let jni = guard.as_ref().ok_or("no jni")?;
        let mut env = jni.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let arr = env.new_object_array(mimes.len() as i32, "java/lang/String", JObject::null()).map_err(|e| e.to_string())?;
        for (i, m) in mimes.iter().enumerate() {
            let s = env.new_string(m).map_err(|e| e.to_string())?;
            env.set_object_array_element(&arr, i as i32, s).map_err(|e| e.to_string())?;
        }
        env.call_static_method(
            &jni.class,
            "pickFiles",
            "(Z[Ljava/lang/String;I)V",
            &[JValue::Bool(multiple as jni::sys::jboolean), JValue::Object(&arr), JValue::Int(token as i32)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Open the system player on an exported file.
    pub fn preview(path: &str) {
        call_str("preview", path);
    }

    /// Open the system share sheet for an exported file.
    pub fn share(path: &str) {
        call_str("share", path);
    }

    fn call_str(method: &str, path: &str) {
        let res = (|| -> Result<(), String> {
            let guard = lock(&JNI);
            let jni = guard.as_ref().ok_or("no jni")?;
            let mut env = jni.vm.attach_current_thread().map_err(|e| e.to_string())?;
            let s = env.new_string(path).map_err(|e| e.to_string())?;
            env.call_static_method(&jni.class, method, "(Ljava/lang/String;)V", &[JValue::Object(&s)]).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(e) = res {
            log::warn!("platform {method}: {e}");
        }
    }
}

#[cfg(target_os = "android")]
pub use bridge::{install, pick_files, preview, share};
