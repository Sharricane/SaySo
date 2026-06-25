// 一次性：生成 1024 的 app 图标 PNG（苹果蓝 squircle + 三根白色声波条）。
// 跑：cargo run --example makeicon  → /tmp/sayso_icon.png，再 sips/iconutil 转 .icns。
use image::{Rgba, RgbaImage};

const N: u32 = 1024;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// 圆角矩形的有符号距离（<0 在内部）。
fn rrect_sd(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - r
}

fn blend(img: &mut RgbaImage, x: u32, y: u32, c: [u8; 3], a: f32) {
    let p = img.get_pixel(x, y).0;
    let bg = [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32 / 255.0];
    let a = a.clamp(0.0, 1.0);
    let out_a = a + bg[3] * (1.0 - a);
    let mix = |fg: f32, b: f32| {
        if out_a <= 0.0 { 0.0 } else { (fg * a + b * bg[3] * (1.0 - a)) / out_a }
    };
    img.put_pixel(
        x,
        y,
        Rgba([
            mix(c[0] as f32, bg[0]) as u8,
            mix(c[1] as f32, bg[1]) as u8,
            mix(c[2] as f32, bg[2]) as u8,
            (out_a * 255.0) as u8,
        ]),
    );
}

fn main() {
    let mut img = RgbaImage::from_pixel(N, N, Rgba([0, 0, 0, 0]));
    let n = N as f32;
    let margin = 96.0; // 像其它 macOS 图标那样留边
    let cx = n / 2.0;
    let cy = n / 2.0;
    let hw = (n - margin * 2.0) / 2.0;
    let r = 200.0; // 圆角

    // squircle + 竖直渐变（上浅蓝→下苹果蓝）
    for y in 0..N {
        for x in 0..N {
            let d = rrect_sd(x as f32 + 0.5, y as f32 + 0.5, cx, cy, hw, hw, r);
            let cov = (0.5 - d).clamp(0.0, 1.0); // 1px 抗锯齿
            if cov > 0.0 {
                let t = y as f32 / n;
                let col = [
                    lerp(56.0, 10.0, t) as u8,
                    lerp(160.0, 122.0, t) as u8,
                    lerp(255.0, 245.0, t) as u8,
                ];
                blend(&mut img, x, y, col, cov);
            }
        }
    }

    // 三根白色声波条（圆角竖条），居中
    let bw = 116.0;
    let gap = 74.0;
    let heights = [330.0_f32, 500.0, 390.0];
    for (i, h) in heights.iter().enumerate() {
        let slot = i as f32 - 1.0;
        let bx = cx + slot * (bw + gap);
        let bhw = bw / 2.0;
        let bhh = h / 2.0;
        for y in 0..N {
            for x in 0..N {
                let d = rrect_sd(x as f32 + 0.5, y as f32 + 0.5, bx, cy, bhw, bhh, bhw);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    blend(&mut img, x, y, [255, 255, 255], cov);
                }
            }
        }
    }

    img.save("/tmp/sayso_icon.png").expect("save png");
    eprintln!("wrote /tmp/sayso_icon.png");
}
