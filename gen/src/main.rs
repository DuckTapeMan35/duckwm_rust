mod api;

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let proto = env::var_os("DUCKWM_PROTO")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let src = env::var_os("DUCKWM_SRC").expect("set DUCKWM_SRC to your duckwm checkout");
            PathBuf::from(src).join("proto/duckwm.proto")
        });
    let dir = proto.parent().expect("schema path has a directory");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src");

    let fds = protox::compile([&proto], [dir]).expect("parse the schema");
    for (file, text) in [
        ("cmd.rs", api::generate_cmd(&fds)),
        ("api.rs", api::generate_api(&fds)),
    ] {
        let path = src.join(file);
        fs::write(&path, text).expect("write the generated file");
        // One item per line comes out long; rustfmt makes it read like the rest.
        let _ = Command::new("rustfmt")
            .args(["--edition", "2024"])
            .arg(&path)
            .status();
    }
    prost_build::Config::new()
        .out_dir(src.join("proto"))
        .compile_fds(fds)
        .expect("generate the bindings");
    println!("generated from {}", proto.display());
}
