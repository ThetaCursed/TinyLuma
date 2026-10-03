// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Embeds the Windows application icon (`assets/icon.ico`) into the executable,
//! so Explorer, the taskbar and Alt-Tab show it without a `.rc` file.
//!
//! Only runs when targeting Windows; other platforms ignore it. Regenerate the
//! icon with `cargo run --release --example gen_icon`.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.set("ProductName", "TinyLuma");
    res.set("FileDescription", "TinyLuma — Post-processing for AI images");
    res.set("LegalCopyright", "Copyright (C) 2026 ThetaCursed");
    res.set("OriginalFilename", "TinyLuma.exe");
    if let Ok(version) = std::env::var("CARGO_PKG_VERSION") {
        res.set("FileVersion", &version);
        res.set("ProductVersion", &version);
    }
    if let Err(err) = res.compile() {
        // Do not break `cargo check`/CI on machines without a resource
        // compiler; the icon is cosmetic, not required to build.
        println!("cargo:warning=failed to embed Windows icon: {err}");
    }
}
