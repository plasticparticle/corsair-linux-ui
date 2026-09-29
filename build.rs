use std::{env, process::Command};

fn main() {
    for path in [
        "src",
        "ui",
        "Cargo.toml",
        "Cargo.lock",
        "tauri.conf.json",
        "build.rs",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let mut date = Command::new("date");
    date.arg("-u");
    if let Ok(epoch) = env::var("SOURCE_DATE_EPOCH") {
        let epoch: u64 = epoch
            .parse()
            .expect("SOURCE_DATE_EPOCH must be Unix seconds");
        date.arg(format!("--date=@{epoch}"));
    }
    let output = date
        .arg("+%Y-%m-%d %H:%M:%S UTC")
        .output()
        .expect("date is required to embed the build timestamp");
    assert!(
        output.status.success(),
        "could not determine the build timestamp"
    );
    let timestamp = String::from_utf8(output.stdout).expect("date output must be UTF-8");
    println!("cargo:rustc-env=CORSAIR_BUILD_DATE={}", timestamp.trim());
    tauri_build::build()
}
