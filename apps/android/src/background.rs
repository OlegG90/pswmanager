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

use jni::objects::{Global, JByteArray, JClass, JObject, JString, JValue, JValueOwned};
use jni::signature::{MethodSignature, RuntimeMethodSignature};
use jni::strings::JNIString;
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::{Env, EnvUnowned, JavaVM, Outcome};
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
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    context: JObject<'local>,
    state: JString<'local>,
) -> jboolean {
    let done = native(&mut env, false, |env| matches!(upload(env, &context, &state), Ok(true)));
    if done {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// Runs `work` with the [Env] of a native method's call (jni 0.22); `fallback`
/// when JNI cannot give one or `work` panics.
pub(crate) fn native<'local, T>(env: &mut EnvUnowned<'local>, fallback: T, work: impl FnOnce(&mut Env<'local>) -> T) -> T {
    match env.with_env(|env| Ok::<T, jni::errors::Error>(work(env))).into_outcome() {
        Outcome::Ok(value) => value,
        _ => fallback,
    }
}

/// A Java string as Rust's.
pub(crate) fn text(env: &Env, value: &JString) -> Result<String, String> {
    Ok(env.get_string(value).map_err(|e| e.to_string())?.to_string())
}

/// Whether the upload is settled ([sync::settled]): WorkManager tries again
/// only when it is not.
///
/// Without the app the work has its own `Store` on the state file. Should the
/// app start in this process meanwhile, both write the file whole; the
/// app's may put back an older sync state, which costs a needless check or
/// merge at the next sync, never data.
fn upload(env: &mut Env, context: &JObject, state: &JString) -> Result<bool, String> {
    if let Some(app) = APP.get() {
        return Ok(crate::app::upload_pending_now(app));
    }
    let state = text(env, state)?;
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
    context: Global<JObject<'static>>,
    keystore: Global<JClass<'static>>,
    document_io: Global<JClass<'static>>,
}

/// An argument after the app's context.
enum Arg<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

impl Kotlin {
    /// Installs the core's secrets and documents through JNI, once.
    pub(crate) fn install(env: &mut Env, context: &JObject) -> Result<(), String> {
        static INSTALLED: OnceLock<()> = OnceLock::new();
        if INSTALLED.get().is_some() {
            return Ok(());
        }
        let class = |env: &mut Env, name: &str| -> Result<Global<JClass<'static>>, String> {
            let class = env.find_class(JNIString::from(name)).map_err(|e| e.to_string())?;
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
    /// with `read`; a Java exception is caught (by the attachment) and becomes
    /// the error.
    fn call<T>(
        &self,
        class: &Global<JClass<'static>>,
        method: &str,
        signature: &str,
        args: &[Arg],
        read: impl FnOnce(&mut Env, JValueOwned) -> jni::errors::Result<T>,
    ) -> Result<T, String> {
        let name = JNIString::from(method);
        let signature: RuntimeMethodSignature = signature.parse().map_err(|e: jni::errors::Error| e.to_string())?;
        self.vm
            .attach_current_thread(|env| {
                env.with_local_frame(8, |env| -> jni::errors::Result<T> {
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
                    let value = env.call_static_method(&**class, &name, MethodSignature::from(&signature), &values)?;
                    read(env, value)
                })
            })
            .map_err(|e: jni::errors::Error| format!("{method} failed: {e}"))
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
                let value = env.cast_local::<JString>(value)?;
                Ok(Some(Zeroizing::new(env.get_string(&value)?.to_string())))
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
            let value = env.cast_local::<JByteArray>(value)?;
            env.convert_byte_array(&value).map(Some)
        })
    }

    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        let signature = "(Landroid/content/Context;Ljava/lang/String;[B)V";
        self.0.call(&self.0.document_io, "write", signature, &[Arg::Text(uri), Arg::Bytes(bytes)], |_, _| Ok(()))
    }
}
