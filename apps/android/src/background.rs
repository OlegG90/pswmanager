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
pub(crate) static APP: OnceLock<AppHandle> = OnceLock::new();

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

/// Whether the upload is settled ([sync::settled]): WorkManager tries again
/// only when it is not.
///
/// Without the app the work has its own `Store` on the state file. Should the
/// app start in this process meanwhile, both write the file whole; the
/// app's may put back an older sync state, which costs a needless check or
/// merge at the next sync, never data.
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
    Ok(sync::settled(&sync::sync(location.open().as_ref(), &store, &Session::default())))
}

/// The Kotlin side, reached from any thread: the classes are found once, on
/// the worker's (or the credential provider's) thread, whose class loader knows them. Installed before the
/// app's own plugins (the work started the process), these serve the app too.
pub(crate) struct Kotlin {
    vm: JavaVM,
    context: GlobalRef,
    keystore: GlobalRef,
    document_io: GlobalRef,
}

/// An argument after the app's context.
enum Arg<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

impl Kotlin {
    /// Installs the core's secrets and documents through JNI, once.
    pub(crate) fn install(env: &mut JNIEnv, context: &JObject) -> Result<(), String> {
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
            document_io: class(env, "io/github/olegg90/pswmanager/DocumentIo")?,
        });
        secrets::install(Box::new(Secrets(kotlin.clone())));
        documents::install(Box::new(Documents(kotlin)));
        let _ = INSTALLED.set(());
        Ok(())
    }

    /// Calls `class`'s static `method` with the app's context and `args`, in a
    /// local frame (a whole database can pass through), and reads the answer
    /// with `read`; a Java exception is cleared and becomes the error.
    fn call<T>(
        &self,
        class: &GlobalRef,
        method: &str,
        signature: &str,
        args: &[Arg],
        read: impl FnOnce(&mut JNIEnv, JValueOwned) -> jni::errors::Result<T>,
    ) -> Result<T, String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let result = env.with_local_frame(8, |env| -> jni::errors::Result<T> {
            let mut objects = Vec::with_capacity(args.len());
            for arg in args {
                objects.push(match arg {
                    Arg::Text(text) => JObject::from(env.new_string(text)?),
                    Arg::Bytes(bytes) => JObject::from(env.byte_array_from_slice(bytes)?),
                });
            }
            let mut values = vec![JValue::Object(self.context.as_obj())];
            for object in &objects {
                values.push(JValue::Object(object));
            }
            let value = env.call_static_method(<&JClass>::from(class.as_obj()), method, signature, &values)?;
            read(env, value)
        });
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
        let signature = "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V";
        self.0.call(&self.0.keystore, "write", signature, &[Arg::Text(name), Arg::Text(secret)], |_, _| Ok(()))
    }

    fn read(&self, name: &str) -> Option<Zeroizing<String>> {
        let signature = "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;";
        self.0
            .call(&self.0.keystore, "read", signature, &[Arg::Text(name)], |env, value| {
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
        let signature = "(Landroid/content/Context;Ljava/lang/String;)V";
        let _ = self.0.call(&self.0.keystore, "delete", signature, &[Arg::Text(name)], |_, _| Ok(()));
    }
}

struct Documents(Arc<Kotlin>);

impl DocumentStore for Documents {
    fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String> {
        let signature = "(Landroid/content/Context;Ljava/lang/String;)[B";
        self.0.call(&self.0.document_io, "read", signature, &[Arg::Text(uri)], |env, value| {
            let value = value.l()?;
            if value.is_null() {
                return Ok(None);
            }
            env.convert_byte_array(JByteArray::from(value)).map(Some)
        })
    }

    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        let signature = "(Landroid/content/Context;Ljava/lang/String;[B)V";
        self.0.call(&self.0.document_io, "write", signature, &[Arg::Text(uri), Arg::Bytes(bytes)], |_, _| Ok(()))
    }
}
