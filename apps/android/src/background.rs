//! The background upload (`UploadWorker.kt`, through JNI): what the app could
//! not send before Android stopped it goes up when WorkManager runs the work.
//! It needs no key: the encrypted working copy is uploaded, conditional on the
//! remote file's revision (`sync::sync` with a locked session); a merge waits
//! for the next unlock.
//!
//! With the app running in this process the upload is the app's own, after a
//! sync that is running. Without it (Android started the process for the
//! work) the core gets its secrets and documents from Kotlin through JNI
//! (`Keystore.kt`, `DocumentIo.kt`) and reads the state file itself; a later
//! start of the app in the same process keeps those, as they are installed
//! first.

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue, JValueOwned};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::{JNIEnv, JavaVM};
use pswm_core::documents::{self, DocumentStore};
use pswm_core::secrets::{self, SecretStore};
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::sync;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tauri::AppHandle;
use zeroize::Zeroizing;

/// The app, when it runs in this process.
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Called at the app's start.
pub fn remember(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_BackgroundUpload_uploadPending<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    context: JObject<'local>,
    state: JString<'local>,
) -> jboolean {
    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| upload(&mut env, &context, &state)));
    if matches!(done, Ok(Ok(true))) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// True when nothing is left to send.
fn upload(env: &mut JNIEnv, context: &JObject, state: &JString) -> Result<bool, String> {
    if let Some(app) = APP.get() {
        return Ok(crate::app::upload_pending_now(app));
    }
    let state: String = env.get_string(state).map_err(|e| e.to_string())?.into();
    Kotlin::install(env, context)?;
    let store = Store::load(PathBuf::from(state));
    if !sync::has_pending(&store) {
        return Ok(true);
    }
    let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else { return Ok(true) };
    // Offline, signed out or changed elsewhere: tried again later, or left for the app.
    let _ = sync::sync(location.open().as_ref(), &store, &Session::default());
    Ok(!sync::has_pending(&store))
}

/// The Kotlin side, reached from any thread: the classes are found once, on
/// the worker's thread, whose class loader knows them.
struct Kotlin {
    vm: JavaVM,
    context: GlobalRef,
    keystore: GlobalRef,
    documents: GlobalRef,
}

const CONTEXT_STRING: &str = "(Landroid/content/Context;Ljava/lang/String;)";

impl Kotlin {
    /// Installs the core's secrets and documents through JNI, once.
    fn install(env: &mut JNIEnv, context: &JObject) -> Result<(), String> {
        static INSTALLED: OnceLock<()> = OnceLock::new();
        if INSTALLED.get().is_some() {
            return Ok(());
        }
        let class = |env: &mut JNIEnv, name: &str| -> Result<GlobalRef, String> {
            let class = env.find_class(name).map_err(|e| e.to_string())?;
            env.new_global_ref(class).map_err(|e| e.to_string())
        };
        let kotlin = Arc::new(Kotlin {
            vm: env.get_java_vm().map_err(|e| e.to_string())?,
            context: env.new_global_ref(context).map_err(|e| e.to_string())?,
            keystore: class(env, "io/github/olegg90/pswmanager/Keystore")?,
            documents: class(env, "io/github/olegg90/pswmanager/DocumentIo")?,
        });
        secrets::install(Box::new(Secrets(kotlin.clone())));
        documents::install(Box::new(Documents(kotlin)));
        let _ = INSTALLED.set(());
        Ok(())
    }

    /// Calls a static method of `class` with the app's context and a string
    /// first; a Java exception is cleared and becomes the error.
    fn call<T>(
        &self,
        class: &GlobalRef,
        method: &str,
        signature: &str,
        text: &str,
        more: Option<&[u8]>,
        answer: impl FnOnce(&mut JNIEnv, JValueOwned) -> jni::errors::Result<T>,
    ) -> Result<T, String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let result = (|| {
            let text = env.new_string(text)?;
            let bytes = more.map(|bytes| env.byte_array_from_slice(bytes)).transpose()?;
            let mut args = vec![JValue::Object(self.context.as_obj()), JValue::Object(&text)];
            if let Some(bytes) = &bytes {
                args.push(JValue::Object(bytes));
            }
            let value = env.call_static_method(<&JClass>::from(class.as_obj()), method, signature, &args)?;
            answer(&mut env, value)
        })();
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err(format!("{method} failed"));
        }
        result.map_err(|e| e.to_string())
    }
}

struct Secrets(Arc<Kotlin>);

impl SecretStore for Secrets {
    fn write(&self, name: &str, secret: &str) -> Result<(), String> {
        let k = &self.0;
        let signature = "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V";
        let mut env = k.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let result = (|| {
            let (name, secret) = (env.new_string(name)?, env.new_string(secret)?);
            let args = [JValue::Object(k.context.as_obj()), JValue::Object(&name), JValue::Object(&secret)];
            env.call_static_method(<&JClass>::from(k.keystore.as_obj()), "write", signature, &args).map(|_| ())
        })();
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err("Cannot keep the secret".into());
        }
        result.map_err(|e| e.to_string())
    }

    fn read(&self, name: &str) -> Option<Zeroizing<String>> {
        let signature = format!("{CONTEXT_STRING}Ljava/lang/String;");
        self.0
            .call(&self.0.keystore, "read", &signature, name, None, |env, value| {
                let value = value.l()?;
                if value.is_null() {
                    return Ok(None);
                }
                Ok(Some(Zeroizing::new(env.get_string(&JString::from(value))?.into())))
            })
            .ok()
            .flatten()
    }

    fn delete(&self, name: &str) {
        let signature = format!("{CONTEXT_STRING}V");
        let _ = self.0.call(&self.0.keystore, "delete", &signature, name, None, |_, _| Ok(()));
    }
}

struct Documents(Arc<Kotlin>);

impl DocumentStore for Documents {
    fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String> {
        let signature = format!("{CONTEXT_STRING}[B");
        self.0.call(&self.0.documents, "read", &signature, uri, None, |env, value| {
            let value = value.l()?;
            if value.is_null() {
                return Ok(None);
            }
            env.convert_byte_array(JByteArray::from(value)).map(Some)
        })
    }

    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        let signature = "(Landroid/content/Context;Ljava/lang/String;[B)V";
        self.0.call(&self.0.documents, "write", signature, uri, Some(bytes), |_, _| Ok(()))
    }
}
