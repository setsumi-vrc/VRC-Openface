//! Embeds the application icon and version information in the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // A tongue model placed in models/ at build time is compiled into the executable.
    // The file is not part of the repository, so builds without it still work.
    println!("cargo:rerun-if-changed=models/tongue_model.onnx");
    println!("cargo::rustc-check-cfg=cfg(embedded_tongue)");
    if std::path::Path::new("models/tongue_model.onnx").is_file() {
        println!("cargo:rustc-cfg=embedded_tongue");
    }
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico").set("ProductName", "VRC Openface").set("FileDescription", "VRC Openface");
        if let Err(e) = resource.compile() {
            // A missing resource compiler only costs the icon; the build still works.
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
