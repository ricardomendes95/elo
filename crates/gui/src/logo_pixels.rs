// Renders the Elo mark (two interlocking links) to RGBA pixels. Shared by the
// runtime window icon (`main.rs`) and the embedded `.exe` icon (`build.rs`),
// so both stay identical. Pure std — no external crates — so `build.rs` can
// `include!` it.

/// Render the logo as `size`×`size` RGBA8 (transparent background).
pub fn render_icon(size: u32) -> Vec<u8> {
    let n = size as usize;
    let mut buf = vec![0u8; n * n * 4]; // transparent
    let s = size as f32;
    let r = s * 0.26; // ring radius
    let w = r * 0.42; // ring stroke width
    let cy = s * 0.5;
    let lc = (s * 0.37, cy); // left ring center
    let rc = (s * 0.63, cy); // right ring center
    let teal = [28u8, 200, 170];
    let indigo = [96u8, 132, 246];

    for y in 0..n {
        for x in 0..n {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dl = (((px - lc.0).powi(2) + (py - lc.1).powi(2)).sqrt() - r).abs();
            let dr = (((px - rc.0).powi(2) + (py - rc.1).powi(2)).sqrt() - r).abs();
            let i = (y * n + x) * 4;
            // 1px anti-aliased edge.
            let edge = w * 0.5;
            let aa = 1.0;
            let cover = |d: f32| -> f32 {
                if d <= edge {
                    1.0
                } else if d <= edge + aa {
                    1.0 - (d - edge) / aa
                } else {
                    0.0
                }
            };
            // Right ring drawn on top of the left where they overlap.
            let (color, a) = if cover(dr) > 0.0 {
                (teal, cover(dr))
            } else if cover(dl) > 0.0 {
                (indigo, cover(dl))
            } else {
                continue;
            };
            buf[i] = color[0];
            buf[i + 1] = color[1];
            buf[i + 2] = color[2];
            buf[i + 3] = (a.clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    buf
}
