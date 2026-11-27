//! Safe JNI value conversion; never expose a private key or passphrase.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
};

use anyhow::{Result, bail, ensure};
use intralink_core::Role;
use intralink_vault::{Profile, Vault};
use jni::{
    JNIEnv,
    objects::{JByteArray, JObject, JString},
    sys::{jint, jstring},
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

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
