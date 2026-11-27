//! Safe JNI value conversion; never expose a private key or passphrase.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::Mutex,
};

use anyhow::{Result, bail, ensure};
use intralink_core::Role;
use intralink_vault::{Profile, Vault};
use jni::{
    JNIEnv,
    objects::{GlobalRef, JByteArray, JObject, JString},
    sys::{jboolean, jint, jlong, jstring},
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::session::{self, Registry};

static SESSIONS: Registry = Registry::new();
static APPLICATION_CONTEXT: Mutex<Option<GlobalRef>> = Mutex::new(None);

fn role(value: jint) -> Result<Role> {
    match value {
        0 => Ok(Role::Controller),
        1 => Ok(Role::Host),
        _ => bail!("Choose BASE or CLIENT"),
    }
}

fn summary(profile: &Profile) -> Value {
    json!({
        "ok": true,
        "role": match profile.role() {
            Role::Controller => "BASE",
            Role::Host => "CLIENT",
            Role::Hybrid => "HYBRID",
        },
        "name": profile.device_name().unwrap_or("Local device"),
        "public_id": profile.public_id().to_string(),
        "peer_count": profile.peers().len(),
        "peers": profile.peers(),
    })
}

fn inputs(
    env: &mut JNIEnv,
    path: &JString,
    password: &JByteArray,
) -> Result<(String, Zeroizing<Vec<u8>>)> {
    let path: String = env.get_string(path)?.into();
    let len = env.get_array_length(password)?;
    ensure!(
        (12..=1024).contains(&len),
        "Use a passphrase of 12-1024 UTF-8 bytes"
    );
    Ok((path, Zeroizing::new(env.convert_byte_array(password)?)))
}

fn reply(env: &mut JNIEnv, operation: impl FnOnce(&mut JNIEnv) -> Result<Value>) -> jstring {
    let result = match catch_unwind(AssertUnwindSafe(|| operation(env))) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => json!({"ok": false, "error": error.to_string()}),
        Err(_) => json!({"ok": false, "error": "Local vault operation aborted"}),
    };
    env.new_string(result.to_string())
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_createProfile(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    selected: jint,
    name: JString,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let (path, password) = inputs(env, &path, &password)?;
        let name: String = env.get_string(&name)?.into();
        let profile = Profile::new_named(role(selected)?, &name)?;
        let vault = Vault::lock(Path::new(&path))?;
        vault.initialize(&profile, &password)?;
        Ok(summary(&profile))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_openProfile(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let (path, password) = inputs(env, &path, &password)?;
        let vault = Vault::lock(Path::new(&path))?;
        Ok(summary(&vault.load(&password)?))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_changeRole(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    selected: jint,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let (path, password) = inputs(env, &path, &password)?;
        let vault = Vault::lock(Path::new(&path))?;
        let profile = vault.load(&password)?;
        let impact = profile.role_change_impact(role(selected)?);
        vault.change_role(&password, impact)?;
        let mut result = summary(&vault.load(&password)?);
        result["revoked_grants"] = json!(impact.revoked_grants);
        Ok(result)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_renameDevice(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    name: JString,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let (path, password) = inputs(env, &path, &password)?;
        let name: String = env.get_string(&name)?.into();
        let vault = Vault::lock(Path::new(&path))?;
        vault.rename_device(&password, &name)?;
        Ok(summary(&vault.load(&password)?))
    })
}

// This is called with Android's applicationContext, not an Activity. The
// global reference keeps the object valid until the process exits. Serialize
// the one-time installation before any Iroh endpoint may be constructed.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_initializeNetwork(
    env: JNIEnv,
    _receiver: JObject,
    context: JObject,
) -> jboolean {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let mut stored = APPLICATION_CONTEXT
            .lock()
            .map_err(|_| anyhow::anyhow!("Network context unavailable"))?;
        if stored.is_none() {
            ensure!(!context.is_null(), "Missing Android application context");
            let vm = env.get_java_vm()?;
            let global = env.new_global_ref(context)?;
            // SAFETY: JavaVM is process-owned and the GlobalRef is retained in
            // this process-lifetime static. Installation is serialized once.
            unsafe {
                iroh::dns::install_android_jni_context(
                    vm.get_java_vm_pointer().cast(),
                    global.as_obj().as_raw().cast(),
                );
            }
            *stored = Some(global);
        }
        Ok(())
    }));
    u8::from(matches!(result, Ok(Ok(()))))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_pairDevice(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    request: JString,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let (path, password) = inputs(env, &path, &password)?;
        let request: String = env.get_string(&request)?.into();
        let profile = session::pair(Path::new(&path), &password, session::decode(&request)?)?;
        Ok(summary(&profile))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_revokeDevice(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    name: JString,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let (path, password) = inputs(env, &path, &password)?;
        let name: String = env.get_string(&name)?.into();
        let profile = session::revoke(Path::new(&path), &password, &name)?;
        Ok(summary(&profile))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_reserveProbe(
    _env: JNIEnv,
    _receiver: JObject,
) -> jlong {
    SESSIONS.reserve().unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_cancelProbe(
    _env: JNIEnv,
    _receiver: JObject,
    id: jlong,
) {
    SESSIONS.cancel(id);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_runProbe(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    request: JString,
    id: jlong,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let lease = SESSIONS.claim(id)?;
        ensure!(
            APPLICATION_CONTEXT
                .lock()
                .map_err(|_| anyhow::anyhow!("Network context unavailable"))?
                .is_some(),
            "Android network context was not initialized"
        );
        let (path, password) = inputs(env, &path, &password)?;
        let request: String = env.get_string(&request)?.into();
        let (profile, elapsed_ms) = session::run(
            lease,
            Path::new(&path),
            password,
            session::decode(&request)?,
        )?;
        let mut result = summary(&profile);
        result["probe_elapsed_ms"] = json!(elapsed_ms);
        Ok(result)
    })
}

/// Explicit opt-in instrumentation build only. This symbol is forbidden in
/// normal distribution candidates. No real screen or input is accessed.
#[cfg(feature = "lan-fixture")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeFixtureBridge_runMediaFixture(
    mut env: JNIEnv,
    _receiver: JObject,
    path: JString,
    request: JString,
    id: jlong,
    password: JByteArray,
) -> jstring {
    reply(&mut env, |env| {
        let lease = SESSIONS.claim(id)?;
        ensure!(
            APPLICATION_CONTEXT
                .lock()
                .map_err(|_| anyhow::anyhow!("Network context unavailable"))?
                .is_some(),
            "Android network context was not initialized"
        );
        let (path, password) = inputs(env, &path, &password)?;
        let request: String = env.get_string(&request)?.into();
        let (profile, elapsed_ms, stats) = session::run_media_fixture(
            lease,
            Path::new(&path),
            password,
            session::decode(&request)?,
        )?;
        let mut result = summary(&profile);
        result["probe_elapsed_ms"] = json!(elapsed_ms);
        result["frames"] = json!(stats.frames);
        result["media_bytes"] = json!(stats.media_bytes);
        Ok(result)
    })
}
