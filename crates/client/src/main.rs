//! Elo client entry point. The real Windows backends (DXGI capture, SendInput)
//! live in the `windows_backend` module, compiled only on Windows. On other
//! platforms the binary still builds (so the workspace is CI-checkable on
//! Linux) but refuses to run the host role, which needs a Windows screen.

mod cli;

fn main() -> anyhow::Result<()> {
    cli::run()
}
