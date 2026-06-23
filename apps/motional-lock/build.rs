fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/motional-icon.ico");
    resource.set("FileDescription", "Motional");
    resource.set("ProductName", "Motional");

    if let Err(error) = resource.compile() {
        panic!("failed to compile Windows resources: {error}");
    }
}
