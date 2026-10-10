//! Writes into the shared Documents folder through MediaStore, which needs no storage permission on Android 10+.

use crate::backup::Sink;
use jni::objects::JObject;
use jni::JNIEnv;

pub struct MediaStore;

fn with_env<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let out = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return Err("Android refused the backup file".into());
    }
    out.map_err(|e| e.to_string())
}

fn resolver<'a>(env: &mut JNIEnv<'a>, ctx: &JObject) -> jni::errors::Result<JObject<'a>> {
    env.call_method(ctx, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()
}

fn parse_uri<'a>(env: &mut JNIEnv<'a>, uri: &str) -> jni::errors::Result<JObject<'a>> {
    let s = env.new_string(uri)?;
    env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[(&s).into()])?.l()
}

fn put(env: &mut JNIEnv, values: &JObject, key: &str, value: &str) -> jni::errors::Result<()> {
    let k = env.new_string(key)?;
    let v = env.new_string(value)?;
    env.call_method(values, "put", "(Ljava/lang/String;Ljava/lang/String;)V", &[(&k).into(), (&v).into()])?;
    Ok(())
}

// "wt" truncates, so a shorter zip doesn't leave stale bytes behind
fn write_to(env: &mut JNIEnv, resolver: &JObject, uri: &JObject, bytes: &[u8]) -> jni::errors::Result<()> {
    let mode = env.new_string("wt")?;
    let stream = env
        .call_method(
            resolver,
            "openOutputStream",
            "(Landroid/net/Uri;Ljava/lang/String;)Ljava/io/OutputStream;",
            &[uri.into(), (&mode).into()],
        )?
        .l()?;
    let arr = env.byte_array_from_slice(bytes)?;
    let written = env.call_method(&stream, "write", "([B)V", &[(&arr).into()]);
    env.call_method(&stream, "close", "()V", &[])?;
    written.map(|_| ())
}

impl Sink for MediaStore {
    fn create(&mut self, name: &str, bytes: &[u8]) -> Result<String, String> {
        with_env(|env, ctx| {
            let resolver = resolver(env, ctx)?;
            let values = env.new_object("android/content/ContentValues", "()V", &[])?;
            put(env, &values, "_display_name", name)?;
            put(env, &values, "mime_type", "application/zip")?;
            put(env, &values, "relative_path", "Documents/Dayfile")?;
            let volume = env.new_string("external")?;
            let collection = env
                .call_static_method(
                    "android/provider/MediaStore$Files",
                    "getContentUri",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[(&volume).into()],
                )?
                .l()?;
            let item = env
                .call_method(
                    &resolver,
                    "insert",
                    "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
                    &[(&collection).into(), (&values).into()],
                )?
                .l()?;
            if item.is_null() {
                return Err(jni::errors::Error::NullPtr("MediaStore insert returned null"));
            }
            if let Err(e) = write_to(env, &resolver, &item, bytes) {
                let _ = env.exception_clear();
                let none = JObject::null();
                let _ = env.call_method(
                    &resolver,
                    "delete",
                    "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
                    &[(&item).into(), (&none).into(), (&none).into()],
                );
                return Err(e);
            }
            let text = env.call_method(&item, "toString", "()Ljava/lang/String;", &[])?.l()?;
            let text = jni::objects::JString::from(text);
            let uri: String = env.get_string(&text)?.into();
            Ok(uri)
        })
    }

    fn overwrite(&mut self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        with_env(|env, ctx| {
            let resolver = resolver(env, ctx)?;
            let uri = parse_uri(env, uri)?;
            write_to(env, &resolver, &uri, bytes)
        })
    }

    fn remove(&mut self, uri: &str) {
        let _ = with_env(|env, ctx| {
            let resolver = resolver(env, ctx)?;
            let uri = parse_uri(env, uri)?;
            let none = JObject::null();
            env.call_method(
                &resolver,
                "delete",
                "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
                &[(&uri).into(), (&none).into(), (&none).into()],
            )?;
            Ok(())
        });
    }
}

