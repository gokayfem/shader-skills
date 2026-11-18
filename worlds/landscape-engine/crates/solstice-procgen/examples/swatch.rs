//! Writes swatches of library surfaces and leaf atlases as images, for judging them by eye
//! against photographs of the real material.
//!
//! ```text
//! cargo run --release -p solstice-procgen --example swatch -- OUT_DIR SIZE NAME...
//! ```
//!
//! Each `NAME.ppm` holds the colour on the left and the colour lit from the upper left by the
//! normal map on the right, which shows the relief.

use solstice_procgen::foliage::leaf::LeafKind;
use solstice_procgen::surfaces;
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: swatch OUT_DIR SIZE NAME...";
    let out = PathBuf::from(args.next().ok_or(usage)?);
    let size: u32 = args
        .next()
        .ok_or(usage)?
        .parse()
        .map_err(|e| format!("SIZE: {e}"))?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for name in args {
        let surface = surfaces::by_name(&name, 0)
            .or_else(|| LeafKind::by_name(&name).map(|kind| kind.atlas(0)))
            .ok_or_else(|| format!("no surface or leaf atlas called {name}"))?;
        let layer = surface.bake(size);
        let (color, normal) = (&layer.color_mips[0], &layer.normal_mips[0]);
        let n = size as usize;
        let light = glam::Vec3::new(-0.5, 0.5, 0.7).normalize();
        let mut pixels = Vec::with_capacity(n * n * 6);
        for y in 0..n {
            for half in 0..2 {
                for x in 0..n {
                    let i = y * n + x;
                    let rgb = [color[i * 4], color[i * 4 + 1], color[i * 4 + 2]];
                    if half == 0 {
                        pixels.extend_from_slice(&rgb);
                    } else {
                        let nx = normal[i * 2] as f32 / 127.5 - 1.0;
                        let ny = normal[i * 2 + 1] as f32 / 127.5 - 1.0;
                        let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
                        let lit = 0.25 + 0.9 * glam::Vec3::new(nx, ny, nz).dot(light).max(0.0);
                        pixels.extend(rgb.map(|c| (c as f32 * lit).min(255.0) as u8));
                    }
                }
            }
        }
        // How steep the normals typically are, to compare relief against a scan's normal map.
        let mut slopes: Vec<f32> = normal
            .chunks_exact(2)
            .map(|n| {
                let (x, y) = (n[0] as f32 / 127.5 - 1.0, n[1] as f32 / 127.5 - 1.0);
                (x * x + y * y).sqrt() / (1.0 - x * x - y * y).max(1.0e-6).sqrt()
            })
            .collect();
        slopes.sort_by(f32::total_cmp);
        let at = |p: f32| slopes[((slopes.len() - 1) as f32 * p) as usize];
        println!(
            "{name}: normal slope percentiles 25/50/75/95 {:.2} {:.2} {:.2} {:.2}",
            at(0.25),
            at(0.5),
            at(0.75),
            at(0.95)
        );
        let path = out.join(format!("{name}.ppm"));
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        write!(file, "P6\n{} {}\n255\n", n * 2, n).map_err(|e| e.to_string())?;
        file.write_all(&pixels).map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
