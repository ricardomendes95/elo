//! On Windows, generate the Elo `.ico` from the shared logo renderer and embed
//! it as the executable's icon. On other platforms this is a no-op.

fn main() {
    #[cfg(windows)]
    embed_windows_icon();
}

// Pull in the shared pixel renderer as a module so its inner docs / `pub fn`
// are valid (an `include!` inside a fn body would not be).
#[cfg(windows)]
mod logo_src {
    include!("src/logo_pixels.rs");
}

#[cfg(windows)]
fn embed_windows_icon() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("elo.ico");
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [256u32, 64, 48, 32, 16] {
        let rgba = logo_src::render_icon(size);
        let image = ico::IconImage::from_rgba_data(size, size, rgba);
        dir.add_entry(ico::IconDirEntry::encode(&image).expect("encode ico entry"));
    }
    let file = std::fs::File::create(&out).expect("create elo.ico");
    dir.write(file).expect("write elo.ico");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(out.to_str().unwrap());
    res.compile().expect("embed windows icon");
    println!("cargo:rerun-if-changed=src/logo_pixels.rs");
}
