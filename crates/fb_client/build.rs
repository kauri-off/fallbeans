fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/fallbeans.ico");
    println!("cargo:rerun-if-env-changed=FB_COMMIT");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icons/fallbeans.ico");
        res.set("ProductName", "Fall Beans");
        res.set("FileDescription", "Fall Beans");
        if let Err(e) = res.compile() {
            // A release (`cargo xtask dist` stamps FB_COMMIT) must not ship an exe without its icon and version.
            if std::env::var_os("FB_COMMIT").is_some() {
                panic!("no exe icon (rc.exe from the Windows SDK?): {e}");
            }
            println!("cargo:warning=no exe icon: {e}");
        }
    }
}
