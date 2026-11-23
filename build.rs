use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist_dir = manifest_dir.join("dist");

    let ui_src = dist_dir.join("rewind.exe");
    let daemon_src = dist_dir.join("rewind-daemon.exe");

    let ui_dst = out_dir.join("payload_rewind.exe");
    let daemon_dst = out_dir.join("payload_rewind_daemon.exe");

    if ui_src.exists() {
        let _ = fs::copy(&ui_src, &ui_dst);
    } else if !ui_dst.exists() {
        let _ = fs::write(&ui_dst, []);
    }

    if daemon_src.exists() {
        let _ = fs::copy(&daemon_src, &daemon_dst);
    } else if !daemon_dst.exists() {
        let _ = fs::write(&daemon_dst, []);
    }

    println!("cargo:rerun-if-changed=dist/rewind.exe");
    println!("cargo:rerun-if-changed=dist/rewind-daemon.exe");
}
