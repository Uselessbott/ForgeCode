use std::env;
use std::path::PathBuf;

fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc is required");
    env::set_var("PROTOC", &protoc);

    let proto = PathBuf::from("../../app/src/main/proto/forgebox.proto");
    println!("cargo:rerun-if-changed={}", proto.display());

    prost_build::compile_protos(&[proto], &["../../app/src/main/proto"])
        .expect("failed to generate Rust protobuf types");
}
