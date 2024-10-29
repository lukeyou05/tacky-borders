fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("src/resources/icon.ico");
    res.compile().unwrap();
}
