//! Embeds the icon, the version information and the application manifest
//! (Common Controls 6, per-monitor DPI v2) into the executable, with the
//! resource compiler llvm-mingw ships (`windres`).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let assets = manifest_dir.join("assets");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let (major, minor, patch) = (
        parts[0],
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    );

    let path = |p: PathBuf| p.display().to_string().replace('\\', "/");
    let rc = format!(
        r#"#include <winver.h>
1 ICON "{icon}"
1 24 "{manifest}"
VS_VERSION_INFO VERSIONINFO
 FILEVERSION {major},{minor},{patch},0
 PRODUCTVERSION {major},{minor},{patch},0
 FILEOS VOS_NT_WINDOWS32
 FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "ScreenShotCM"
      VALUE "FileDescription", "ScreenShotCM - screenshots and annotation"
      VALUE "LegalCopyright", "Copyright (C) 2026 cmonroyp. GPL-3.0-or-later."
      VALUE "Comments", "Open source: https://github.com/screenshotcm/ScreenShotCM"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "screenshotcm"
      VALUE "OriginalFilename", "screenshotcm.exe"
      VALUE "ProductName", "ScreenShotCM"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = path(assets.join("screenshotcm.ico")),
        manifest = path(assets.join("app.manifest")),
    );
    let rc_path = out.join("screenshotcm.rc");
    fs::write(&rc_path, rc).unwrap();
    let obj = out.join("screenshotcm-res.o");
    let status = Command::new("x86_64-w64-mingw32-windres")
        .arg(&rc_path)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("windres from llvm-mingw must be on PATH (scripts/cargo.ps1 puts it there)");
    assert!(status.success(), "windres failed");
    // Linked into the binary only: tests and build scripts need no icon.
    println!("cargo:rustc-link-arg-bins={}", obj.display());
    println!("cargo:rerun-if-changed=assets/screenshotcm.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=build.rs");
}
