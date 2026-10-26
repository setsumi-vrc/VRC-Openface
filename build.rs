//! Embeds the application icon and version information in the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
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
