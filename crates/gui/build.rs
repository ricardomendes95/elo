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
    set_version_info(&mut res, "Elo", "Elo - acesso remoto", "elo.exe");
    res.compile().expect("embed windows icon");
    println!("cargo:rerun-if-changed=src/logo_pixels.rs");
}

/// Version information and manifest: an executable with a product name, a
/// description and an explicit `asInvoker` manifest looks like ordinary
/// software to antivirus heuristics (and shows proper details in Explorer).
#[cfg(windows)]
fn set_version_info(res: &mut winresource::WindowsResource, product: &str, description: &str, file: &str) {
    res.set("ProductName", product)
        .set("FileDescription", description)
        .set("CompanyName", "Ricardo Mendes")
        .set("LegalCopyright", "Copyright (c) 2026 Ricardo Mendes")
        .set("OriginalFilename", file)
        .set("InternalName", file)
        .set_language(0x0416)
        .set_manifest(&manifest());
}

/// Manifesto com a arquitetura do alvo (x86 para 32 bits, amd64 para 64).
#[cfg(windows)]
fn manifest() -> String {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "amd64",
    };
    MANIFEST.replace("{ARCH}", arch)
}

#[cfg(windows)]
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Elo.Remote" version="0.3.5.0" processorArchitecture="{ARCH}"/>
  <description>Elo - acesso remoto</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>
"#;
