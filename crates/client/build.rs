//! On Windows, embed version information and a manifest in `elo-client.exe`.
//! On other platforms this is a no-op.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        set_version_info(&mut res, "Elo", "Elo - cliente de linha de comando", "elo-client.exe");
        res.compile().expect("embed windows resources");
    }
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
        .set_manifest(MANIFEST);
}

#[cfg(windows)]
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Elo.Client" version="0.1.0.0" processorArchitecture="amd64"/>
  <description>Elo - cliente de linha de comando</description>
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
