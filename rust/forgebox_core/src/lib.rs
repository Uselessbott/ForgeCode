//! ForgeBox Rust core — Milestone 2 JNI handshake.
//!
//! This is deliberately tiny. M3 replaces the façade with the protobuf runtime
//! described in docs/ARCHITECTURE.md; the JNI symbol names are kept stable.

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

pub const CORE_VERSION: &str = "0.1.0-m2";

/// Pure Rust logic, unit-testable on the host without a JVM.
pub fn greeting(name: &str) -> String {
    format!("ForgeBox core {CORE_VERSION} online — hello, {name}!")
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
pub extern "C" fn Java_dev_forgebox_core_ForgeNative_nativeEcho(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    input: jstring,
) -> jstring {
    let value = unsafe { JString::from_raw(input) };
    let name: String = match env.get_string(&value) {
        Ok(value) => value.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    match env.new_string(greeting(&name)) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_contains_version_and_name() {
        let message = greeting("Ada");
        assert!(message.contains(CORE_VERSION));
        assert!(message.ends_with("hello, Ada!"));
    }

    #[test]
    fn version_is_stable() {
        assert_eq!(CORE_VERSION, "0.1.0-m2");
    }
}
