fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/fallbeans.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icons/fallbeans.ico");
        res.set("ProductName", "Fall Beans");
        res.set("FileDescription", "Fall Beans");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no exe icon: {e}");
        }
    }
}
