use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    println!("cargo:rerun-if-changed=cpp/webengine_init.cpp");
    println!("cargo:rerun-if-changed=cpp/reveal_mac.mm");
    println!("cargo:rerun-if-changed=qml/Shell.qml");

    let mut builder = CxxQtBuilder::new_qml_module(QmlModule::new("com.xiao.shell").qml_file("qml/Shell.qml"))
        .qt_module("Gui")
        .qt_module("Network")
        .qt_module("Qml")
        .qt_module("Quick")
        .qt_module("WebEngineCore")
        .qt_module("WebEngineQuick")
        .qt_module("WebChannel")
        .qt_module("Widgets")
        .files(["src/bridge.rs"]);

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" => {
            builder = builder.qt_module("DBus");
            println!("cargo:rustc-link-lib=dl");
        }
        "macos" => println!("cargo:rustc-link-lib=framework=AppKit"),
        "windows" => {
            println!("cargo:rustc-link-lib=shell32");
            println!("cargo:rustc-link-lib=ole32");
        }
        _ => {}
    }

    let builder = unsafe {
        builder.cc_builder(move |cc| {
            cc.file("cpp/webengine_init.cpp");
            if target_os == "macos" {
                cc.file("cpp/reveal_mac.mm");
            }
        })
    };
    builder.build().export();
}
