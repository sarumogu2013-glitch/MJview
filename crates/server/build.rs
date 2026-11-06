//! Windows 向けのとき、exe にアイコン（assets/icon/janscope.ico）を埋め込む。
fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/janscope.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon/janscope.ico");
        if let Err(e) = res.compile() {
            // アイコンが付かないだけなので止めない
            println!("cargo:warning=アイコンを埋め込めませんでした: {e}");
        }
    }
}
