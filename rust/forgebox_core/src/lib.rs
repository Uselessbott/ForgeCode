//! ForgeBox Rust core — Milestone 3 protobuf façade.
//!
//! Kotlin owns a single `Runtime` handle. Requests and responses are encoded
//! with the schema in `proto/forgebox.proto`; events are polled separately.

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/forgebox.v1.rs"));
}

pub mod error;
pub mod vfs;

mod db;
mod runtime;

pub use runtime::{Runtime, CORE_VERSION};

use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jbyteArray, jint, jlong, jobjectArray, jstring};
use jni::JNIEnv;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn java_string(env: &mut JNIEnv<'_>, value: jstring) -> jni::errors::Result<String> {
    let value = unsafe { JString::from_raw(value) };
    let result = env.get_string(&value)?.into();
    Ok(result)
}

fn throw(env: &mut JNIEnv<'_>, message: &str) {
    let _ = env.throw_new("java/lang/IllegalStateException", message);
}

#[no_mangle]
pub extern "C" fn Java_dev_forgebox_core_ForgeNative_nativeVersion(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    match env.new_string(CORE_VERSION) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn Java_dev_forgebox_core_ForgeNative_nativeInit(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    app_data_dir: jstring,
    native_lib_dir: jstring,
    db_path: jstring,
) -> jlong {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<jlong, String> {
        let app_data_dir = java_string(&mut env, app_data_dir).map_err(|e| e.to_string())?;
        let native_lib_dir = java_string(&mut env, native_lib_dir).map_err(|e| e.to_string())?;
        let db_path = java_string(&mut env, db_path).map_err(|e| e.to_string())?;
        let runtime = Runtime::new(
            std::path::PathBuf::from(app_data_dir),
            std::path::PathBuf::from(native_lib_dir),
            std::path::PathBuf::from(db_path),
        )
        .map_err(|e| e.to_string())?;
        Ok(Box::into_raw(Box::new(runtime)) as jlong)
    }));
    match result {
        Ok(Ok(handle)) => handle,
        Ok(Err(message)) => {
            throw(&mut env, &message);
            0
        }
        Err(_) => {
            throw(&mut env, "Rust core panicked during initialization");
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn Java_dev_forgebox_core_ForgeNative_nativeCall(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    request: jbyteArray,
) -> jbyteArray {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<jbyteArray, String> {
        if handle == 0 {
            return Err("runtime is not initialized".into());
        }
        let runtime = unsafe { &*(handle as *const Runtime) };
        let request = unsafe { JByteArray::from_raw(request) };
        let bytes = env.convert_byte_array(request).map_err(|e| e.to_string())?;
        let response = runtime.call(&bytes);
        let array = env
            .byte_array_from_slice(&response)
            .map_err(|e| e.to_string())?;
        Ok(array.into_raw())
    }));
    match result {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            throw(&mut env, &message);
            std::ptr::null_mut()
        }
        Err(_) => {
            throw(&mut env, "Rust core panicked during call");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn Java_dev_forgebox_core_ForgeNative_nativePollEvents(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    max: jint,
) -> jobjectArray {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<jobjectArray, String> {
        if handle == 0 {
            return Err("runtime is not initialized".into());
        }
        let runtime = unsafe { &*(handle as *const Runtime) };
        let events = runtime.poll_events(max.max(0) as usize);
        let byte_array_class = env.find_class("[B").map_err(|e| e.to_string())?;
        let array = env
            .new_object_array(
                events.len() as jni::sys::jsize,
                byte_array_class,
                JObject::null(),
            )
            .map_err(|e| e.to_string())?;
        for (index, bytes) in events.iter().enumerate() {
            let element = env
                .byte_array_from_slice(bytes)
                .map_err(|e| e.to_string())?;
            env.set_object_array_element(&array, index as jni::sys::jsize, &element)
                .map_err(|e| e.to_string())?;
        }
        Ok(array.into_raw())
    }));
    match result {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            throw(&mut env, &message);
            std::ptr::null_mut()
        }
        Err(_) => {
            throw(&mut env, "Rust core panicked while polling events");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn Java_dev_forgebox_core_ForgeNative_nativeShutdown(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let runtime = unsafe { Box::from_raw(handle as *mut Runtime) };
        drop(runtime);
    }));
    if result.is_err() {
        throw(&mut env, "Rust core panicked during shutdown");
    }
}
