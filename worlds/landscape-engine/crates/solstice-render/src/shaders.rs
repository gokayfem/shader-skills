//! Shader sources and a tiny `#include` preprocessor.
//!
//! WGSL has no module system. Every file names its dependencies with
//! `#include "path"`, and each file is pasted in once per final shader, at its first
//! include. Sources are embedded at compile time, so there is no file I/O at runtime
//! and the browser build needs nothing extra.

use crate::regions::{rewrite_shader, RegionPlan};
use std::collections::HashSet;

macro_rules! shader_table {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_str!(concat!("../shaders/", $path)))),*]
    };
}

/// Every shader file, by its path relative to the `shaders` directory.
pub const SOURCES: &[(&str, &str)] = shader_table![
    "common/math.wgsl",
    "common/noise.wgsl",
    "common/wind.wgsl",
    "common/sway.wgsl",
    "common/sway_vertex.wgsl",
    "wind/field.wgsl",
    "wind/sway_probe.wgsl",
    "rain/params.wgsl",
    "rain/map.wgsl",
    "rain/light.wgsl",
    "rain/ripples.wgsl",
    "rain/draw.wgsl",
    "rain/surface.wgsl",
    "rain/rings.wgsl",
    "lightning/bolt.wgsl",
    "common/scene.wgsl",
    "common/skin.wgsl",
    "common/pool.wgsl",
    "common/skin_vertex.wgsl",
    "common/frame.wgsl",
    "common/wetness.wgsl",
    "ground/ruts.wgsl",
    "common/brdf.wgsl",
    "common/atmosphere.wgsl",
    "clouds/params.wgsl",
    "common/environment.wgsl",
    "clouds/common.wgsl",
    "vg/pass_view.wgsl",
    "vg/vis_write.wgsl",
    "vg/raster_common.wgsl",
    "vg/raster_hw.wgsl",
    "gi/probe_common.wgsl",
    "gi/radiance_codec.wgsl",
    "gi/screen_trace.wgsl",
    "gi/screen_history.wgsl",
    "vg/cull.wgsl",
    "vg/cull_control.wgsl",
    "vg/raster_hw_fragment.wgsl",
    "vg/depth_stamp.wgsl",
    "vg/raster_sw_body.wgsl",
    "vg/raster_sw.wgsl",
    "vg/raster_sw_depth.wgsl",
    "vg/raster_sw_masked.wgsl",
    "vg/raster_sw_depth_masked.wgsl",
    "vg/raster_hw_masked.wgsl",
    "vg/mask.wgsl",
    "vg/cover_mask.wgsl",
    "vg/cover_none.wgsl",
    "vg/depth_write.wgsl",
    "vg/shadow_merge.wgsl",
    "vg/speck_depth.wgsl",
    "vg/hzb_seed.wgsl",
    "vg/hzb_down.wgsl",
    "vg/resolve.wgsl",
    "vg/resolve_depth.wgsl",
    "sky/luts.wgsl",
    "clouds/noise.wgsl",
    "clouds/coverage.wgsl",
    "clouds/shadow.wgsl",
    "clouds/shadow_mean.wgsl",
    "clouds/panorama.wgsl",
    "clouds/screen.wgsl",
    "gi/voxelize.wgsl",
    "gi/jump_flood.wgsl",
    "gi/voxel_light.wgsl",
    "gi/volume_write.wgsl",
    "gi/depth_pyramid.wgsl",
    "gi/probe_trace.wgsl",
    "gi/probe_project.wgsl",
    "gi/probe_integrate.wgsl",
    "gi/reflections.wgsl",
    "ocean/common.wgsl",
    "ocean/fft.wgsl",
    "ocean/spectrum.wgsl",
    "ocean/evolve.wgsl",
    "ocean/fft_rows.wgsl",
    "ocean/fft_columns.wgsl",
    "ocean/mips.wgsl",
    "ocean/surface_params.wgsl",
    "ocean/seabed.wgsl",
    "ocean/waterline_params.wgsl",
    "ocean/waterline.wgsl",
    "ocean/surface.wgsl",
    "ocean/optics.wgsl",
    "ocean/shade.wgsl",
    "ocean/pond.wgsl",
    "sky/milky_way.wgsl",
    "sky/milky_way_bake.wgsl",
    "sky/bodies.wgsl",
    "lighting/sun_shadow.wgsl",
    "lighting/specular_aa.wgsl",
    "lighting/light_tiles_common.wgsl",
    "lighting/light_shadows_common.wgsl",
    "lighting/deferred.wgsl",
    "lighting/light_tiles.wgsl",
    "lighting/light_shadows.wgsl",
    "particles/common.wgsl",
    "particles/light_common.wgsl",
    "particles/plan.wgsl",
    "particles/light.wgsl",
    "particles/simulate.wgsl",
    "particles/sort.wgsl",
    "particles/collide.wgsl",
    "particles/draw.wgsl",
    "cards/draw.wgsl",
    "particles/depth_copy.wgsl",
    "post/taa.wgsl",
    "post/exposure.wgsl",
    "post/bloom.wgsl",
    "post/tonemap.wgsl",
    "editor/pick.wgsl",
];

/// Entry-point files: the ones compiled into shader modules. Everything else exists
/// only to be included.
#[cfg(test)]
pub const ENTRY_POINTS: &[&str] = &[
    "vg/cull.wgsl",
    "editor/pick.wgsl",
    "wind/field.wgsl",
    "wind/sway_probe.wgsl",
    "rain/map.wgsl",
    "rain/light.wgsl",
    "rain/ripples.wgsl",
    "rain/draw.wgsl",
    "lightning/bolt.wgsl",
    "vg/cull_control.wgsl",
    "vg/raster_hw_fragment.wgsl",
    "vg/raster_sw.wgsl",
    "vg/raster_sw_depth.wgsl",
    "vg/raster_sw_masked.wgsl",
    "vg/raster_sw_depth_masked.wgsl",
    "vg/raster_hw_masked.wgsl",
    "vg/shadow_merge.wgsl",
    "vg/speck_depth.wgsl",
    "vg/hzb_seed.wgsl",
    "vg/hzb_down.wgsl",
    "vg/resolve.wgsl",
    "vg/resolve_depth.wgsl",
    "sky/luts.wgsl",
    "ocean/spectrum.wgsl",
    "ocean/evolve.wgsl",
    "ocean/fft_rows.wgsl",
    "ocean/fft_columns.wgsl",
    "ocean/mips.wgsl",
    "ocean/surface.wgsl",
    "ocean/shade.wgsl",
    "ocean/pond.wgsl",
    "ocean/waterline.wgsl",
    "sky/milky_way_bake.wgsl",
    "clouds/noise.wgsl",
    "clouds/coverage.wgsl",
    "clouds/shadow.wgsl",
    "clouds/shadow_mean.wgsl",
    "clouds/panorama.wgsl",
    "clouds/screen.wgsl",
    "gi/voxelize.wgsl",
    "gi/jump_flood.wgsl",
    "gi/voxel_light.wgsl",
    "gi/volume_write.wgsl",
    "gi/depth_pyramid.wgsl",
    "gi/probe_trace.wgsl",
    "gi/probe_project.wgsl",
    "gi/probe_integrate.wgsl",
    "gi/reflections.wgsl",
    "lighting/deferred.wgsl",
    "lighting/light_tiles.wgsl",
    "lighting/light_shadows.wgsl",
    "particles/plan.wgsl",
    "particles/simulate.wgsl",
    "particles/light.wgsl",
    "particles/sort.wgsl",
    "particles/collide.wgsl",
    "particles/draw.wgsl",
    "cards/draw.wgsl",
    "particles/depth_copy.wgsl",
    "post/taa.wgsl",
    "post/exposure.wgsl",
    "post/bloom.wgsl",
    "post/tonemap.wgsl",
];

/// Returns the source of `path` with all includes expanded.
///
/// # Panics
/// If a file is missing from [`SOURCES`]. Shaders are compiled into the binary, so
/// this can only be a programming error, and the test suite covers every entry point.
pub fn expand(path: &str) -> String {
    let mut output = String::new();
    let mut seen = HashSet::new();
    append(path, &mut output, &mut seen);
    output
}

fn append<'a>(path: &'a str, output: &mut String, seen: &mut HashSet<&'a str>) {
    let (name, source) = SOURCES
        .iter()
        .find(|(name, _)| *name == path)
        .unwrap_or_else(|| panic!("unknown shader file: {path}"));
    if !seen.insert(name) {
        return;
    }
    for line in source.lines() {
        match line.trim().strip_prefix("#include ") {
            Some(include) => append(lookup_name(include.trim().trim_matches('"')), output, seen),
            None => {
                output.push_str(line);
                output.push('\n');
            }
        }
    }
}

/// Returns the `'static` table name for an include path, so `seen` can borrow it.
fn lookup_name(path: &str) -> &'static str {
    SOURCES
        .iter()
        .map(|(name, _)| *name)
        .find(|name| *name == path)
        .unwrap_or_else(|| panic!("unknown shader include: {path}"))
}

/// Compiles an entry-point file into a shader module.
pub fn module(device: &wgpu::Device, path: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(path),
        source: wgpu::ShaderSource::Wgsl(expand(path).into()),
    })
}

/// The line in `common/skin.wgsl` that turns skinning on.
const SKINNING_ON: &str = "const SKINNING: bool = true;";
/// The line in `common/sway.wgsl` that turns swaying in the wind on.
const SWAYING_ON: &str = "const SWAYING: bool = true;";

/// What can move the vertices of a scene's instances, which the shaders that place them are
/// compiled for. What a scene doesn't have is compiled out, so it costs what it did before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deform {
    /// Anything bends to a pose (`skin.rs`).
    pub skinned: bool,
    /// Anything sways in the wind (`wind.rs`).
    pub sways: bool,
}

/// The source of an entry point that places vertices, for a scene with or without anything
/// skinned or swaying.
pub fn expand_for(path: &str, deform: Deform, regions: Option<RegionPlan>) -> String {
    let mut source = expand(path);
    if !deform.skinned {
        assert!(source.contains(SKINNING_ON), "{path} doesn't skin");
        source = source.replace(SKINNING_ON, "const SKINNING: bool = false;");
    }
    if !deform.sways {
        assert!(source.contains(SWAYING_ON), "{path} doesn't sway");
        source = source.replace(SWAYING_ON, "const SWAYING: bool = false;");
    }
    match regions {
        Some(plan) => rewrite_shader(&source, plan.cluster_chunk, plan.pools()),
        None => source,
    }
}

/// A shader with the lines between `// glow:begin` and `// glow:end` taken out, as they were
/// never written, for a scene with no emissive maps.
pub fn without_glow(source: &str) -> String {
    without_section(source, "glow")
}

/// A shader with the lines between `// NAME:begin` and `// NAME:end` taken out, as they were
/// never written.
pub fn without_section(source: &str, name: &str) -> String {
    let (begin, end) = (format!("// {name}:begin"), format!("// {name}:end"));
    let mut out = String::with_capacity(source.len());
    let mut inside = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == begin {
            inside = true;
        } else if trimmed == end {
            inside = false;
        } else if !inside {
            out.push_str(line);
        }
    }
    out
}

/// A water shader that reads the opaque scene's depth from mip 0 of its depth pyramid, which
/// holds the same depth, in place of a binding of its own: see
/// [`crate::Fallbacks::ocean_depth_from_pyramid`].
pub fn depth_from_pyramid(source: &str) -> String {
    const LOAD: &str = "textureLoad(opaque_depth,";
    assert!(
        source.contains(LOAD),
        "the shader doesn't read opaque_depth"
    );
    without_section(source, "opaque-depth").replace(LOAD, "textureLoad(depth_pyramid,")
}

/// Compiles an entry-point file that places vertices: see [`expand_for`].
pub fn module_for(
    device: &wgpu::Device,
    path: &str,
    deform: Deform,
    regions: Option<RegionPlan>,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(path),
        source: wgpu::ShaderSource::Wgsl(expand_for(path, deform, regions).into()),
    })
}

/// Compiles an entry-point file that reads the scene's geometry but places no vertices.
pub fn module_regions(
    device: &wgpu::Device,
    path: &str,
    regions: Option<RegionPlan>,
) -> wgpu::ShaderModule {
    let source = expand(path);
    let source = match regions {
        Some(plan) => rewrite_shader(&source, plan.cluster_chunk, plan.pools()),
        None => source,
    };
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(path),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// How naga lays out a struct, for tests that check a Rust twin matches it byte for byte: its
/// size, and each member's name and offset in order. `path` is an entry point that includes
/// the struct.
#[cfg(test)]
pub(crate) fn struct_layout(path: &str, name: &str) -> (usize, Vec<(String, usize)>) {
    let source = expand(path);
    let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{path} has no struct {name}"));
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("{name} is a struct");
    };
    let members = members
        .iter()
        .map(|m| (m.name.clone().unwrap_or_default(), m.offset as usize))
        .collect();
    (*span as usize, members)
}

/// Checks a Rust struct against its WGSL twin: the same size, and each field at the same
/// offset as the shader member in the same place. Fields are listed in order, one per shader
/// member. A Rust array that the shader spells out as several members is listed once for each,
/// with the extra bytes to each: `rows, rows + 16, rows + 32`.
#[cfg(test)]
macro_rules! assert_twin {
    ($path:expr, $wgsl:expr, $rust:ty, [$($field:ident $(+ $extra:literal)?),* $(,)?]) => {{
        let (size, members) = crate::shaders::struct_layout($path, $wgsl);
        assert_eq!(size, std::mem::size_of::<$rust>(), "size of {}", $wgsl);
        let rust: Vec<(&str, usize)> = vec![$((
            stringify!($field),
            std::mem::offset_of!($rust, $field) $(+ $extra)?,
        )),*];
        assert_eq!(rust.len(), members.len(), "member count of {}", $wgsl);
        for ((field, offset), (member, member_offset)) in rust.iter().zip(&members) {
            assert_eq!(offset, member_offset, "{field} against {member} in {}", $wgsl);
        }
    }};
}
#[cfg(test)]
pub(crate) use assert_twin;

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(path: &str) -> Result<(), String> {
        let source = expand(path);
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|error| format!("{path}:\n{}", error.emit_to_string(&source)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .map_err(|error| format!("{path}:\n{}", error.emit_to_string(&source)))?;
        Ok(())
    }

    /// Catches every WGSL error at `cargo test` time, with no GPU required.
    #[test]
    fn all_entry_points_compile() {
        let failures: Vec<String> = ENTRY_POINTS
            .iter()
            .filter_map(|path| validate(path).err())
            .collect();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// Every shader the renderer makes a module of is an entry point, so the tests above see it.
    /// Only calls that name their shader as a literal are seen; one with a path in a variable
    /// must be listed by hand.
    #[test]
    fn every_shader_made_into_a_module_is_listed() {
        let mut unlisted = Vec::new();
        let mut folders = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for call in ["module(", "module_for("] {
                    for (at, _) in text.match_indices(call) {
                        // The first string after the call, within its arguments, is the path.
                        let rest = &text[at + call.len()..];
                        let Some(open) = rest[..rest.len().min(160)].find('"') else {
                            continue;
                        };
                        let name = &rest[open + 1..];
                        let name = &name[..name.find('"').unwrap()];
                        if name.ends_with(".wgsl") && !ENTRY_POINTS.contains(&name) {
                            unlisted.push(format!("{}: {name}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(unlisted.is_empty(), "not in ENTRY_POINTS: {unlisted:?}");
    }

    /// Every shader `module_for` is used on, in the form scenes with nothing skinned get.
    /// A scene with no emissive maps must get the shader it had before they existed.
    #[test]
    fn resolve_without_glow_binds_and_mentions_none_of_it() {
        let source = without_glow(&expand("vg/resolve.wgsl"));
        for gone in ["glow_maps", "glow:begin", "MATERIAL_GLOW", "binding(16)"] {
            assert!(!source.contains(gone), "{gone} is still there");
        }
        let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("the shader is valid");
    }

    fn assert_valid(source: &str) {
        let module = naga::front::wgsl::parse_str(source).expect("the shader parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("the shader is valid");
    }

    /// For a GPU allowing four storage textures per stage, the resolve leaves depth to
    /// vg/resolve_depth.wgsl, and binds what it keeps at the numbers it always had.
    #[test]
    fn resolve_without_depth_binds_four_storage_textures() {
        for glow in [false, true] {
            let mut source = without_section(&expand("vg/resolve.wgsl"), "depth");
            if !glow {
                source = without_glow(&source);
            }
            for gone in ["out_depth", "depth:begin", "@group(1) @binding(4)"] {
                assert!(!source.contains(gone), "{gone} is still there");
            }
            assert_eq!(source.matches("texture_storage_2d").count(), 4);
            assert!(source.contains("@group(1) @binding(5) var material_sampler"));
            assert_valid(&source);
        }
        let full = expand("vg/resolve.wgsl");
        assert_eq!(full.matches("texture_storage_2d").count(), 5);
    }

    /// The ocean read through its depth pyramid binds one sampled texture fewer, and reads
    /// mip 0 wherever it read the scene's depth.
    #[test]
    fn ocean_shade_reads_depth_from_the_pyramid() {
        let full = expand("ocean/shade.wgsl");
        let source = depth_from_pyramid(&full);
        for gone in [
            "var opaque_depth",
            "textureLoad(opaque_depth",
            "opaque-depth:begin",
        ] {
            assert!(!source.contains(gone), "{gone} is still there");
        }
        assert!(!source.contains("@group(0) @binding(1)"));
        assert_eq!(
            source.matches("textureLoad(depth_pyramid,").count(),
            full.matches("textureLoad(opaque_depth,").count()
                + full.matches("textureLoad(depth_pyramid,").count()
        );
        assert_valid(&source);
    }

    #[test]
    fn a_section_and_its_markers_go_and_nothing_else() {
        let source = "a
// x:begin
b
  // x:end
c
// y:begin
d
// y:end
";
        assert_eq!(
            without_section(source, "x"),
            "a
c
// y:begin
d
// y:end
"
        );
    }

    #[test]
    fn the_average_a_glow_map_is_taken_for_matches_its_twin() {
        let expected = format!(
            "const MATERIAL_GLOW_AVERAGE: f32 = {:?};",
            solstice_asset::Material::GLOW_MAP_AVERAGE
        );
        assert!(expand("vg/resolve.wgsl").contains(&expected), "{expected}");
    }

    #[test]
    fn shaders_with_skinning_compiled_out_are_valid() {
        for path in [
            "vg/cull.wgsl",
            "vg/raster_sw.wgsl",
            "vg/raster_hw_fragment.wgsl",
            "vg/raster_sw_depth.wgsl",
            "vg/raster_sw_masked.wgsl",
            "vg/raster_sw_depth_masked.wgsl",
            "vg/raster_hw_masked.wgsl",
            "vg/resolve.wgsl",
        ] {
            let source = expand_for(path, Deform::default(), None);
            let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
            assert!(source.contains("const SKINNING: bool = false;"), "{path}");
            assert!(source.contains("const SWAYING: bool = false;"), "{path}");
        }
    }

    /// Scenes too large for one binding read everything through the rewritten accessors.
    #[test]
    fn shaders_rewritten_for_regions_are_valid() {
        let plan = RegionPlan {
            cluster_chunk: 1000,
            band_slots: [1, 1, 1, 1, 0, 0, 0, 0],
        };
        let check = |path: &str, source: String| {
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error| panic!("{path}: {error}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
            assert!(source.contains("clusters_more"), "{path}");
        };
        for path in [
            "vg/cull.wgsl",
            "vg/raster_sw.wgsl",
            "vg/raster_hw_fragment.wgsl",
            "vg/raster_sw_depth.wgsl",
            "vg/raster_sw_masked.wgsl",
            "vg/raster_sw_depth_masked.wgsl",
            "vg/raster_hw_masked.wgsl",
            "vg/resolve.wgsl",
        ] {
            for skinned in [false, true] {
                for sways in [false, true] {
                    check(
                        path,
                        expand_for(path, Deform { skinned, sways }, Some(plan)),
                    );
                }
            }
        }
        check(
            "gi/voxelize.wgsl",
            rewrite_shader(
                &expand("gi/voxelize.wgsl"),
                plan.cluster_chunk,
                plan.pools(),
            ),
        );
    }

    /// Operators WGSL will not mix without parentheses: a bitwise operator with any other
    /// binary operator, or two different bitwise operators. Some compilers, naga among them,
    /// accept the mix anyway, so validation alone misses it, but browsers reject the shader.
    fn unparenthesised_mixes(source: &str) -> Vec<String> {
        const BITWISE: [&str; 3] = [" ^ ", " | ", " & "];
        const OTHER: [&str; 7] = [" * ", " / ", " % ", " + ", " - ", " << ", " >> "];
        let mut found = Vec::new();
        for (number, line) in source.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            // Text at each depth of parentheses, with nested groups left out.
            let mut levels: Vec<String> = vec![String::new()];
            for c in code.chars() {
                match c {
                    '(' | '[' => {
                        levels.last_mut().unwrap().push(' ');
                        levels.push(String::new());
                    }
                    ')' | ']' => {
                        let inner = levels.pop().unwrap_or_default();
                        check(&inner, number, line, &mut found);
                        if levels.is_empty() {
                            levels.push(String::new());
                        }
                    }
                    _ => levels.last_mut().unwrap().push(c),
                }
            }
            for level in &levels {
                check(level, number, line, &mut found);
            }
        }
        fn check(text: &str, number: usize, line: &str, found: &mut Vec<String>) {
            let text = text.replace("&&", "  ").replace("||", "  ");
            let bitwise: Vec<_> = BITWISE.iter().filter(|op| text.contains(**op)).collect();
            if bitwise.is_empty() {
                return;
            }
            if bitwise.len() > 1 || OTHER.iter().any(|op| text.contains(op)) {
                found.push(format!("line {}: {}", number + 1, line.trim()));
            }
        }
        found
    }

    #[test]
    fn no_shader_mixes_operators_that_browsers_reject() {
        let problems: Vec<String> = ENTRY_POINTS
            .iter()
            .flat_map(|path| {
                unparenthesised_mixes(&expand(path))
                    .into_iter()
                    .map(move |p| format!("{path}: {p}"))
            })
            .collect();
        assert!(
            problems.is_empty(),
            "
{}",
            problems.join(
                "
"
            )
        );
        // And it does catch the mistake that once left the browser's sky black.
        assert_eq!(unparenthesised_mixes("let h = a * 3u ^ b * 5u;").len(), 1);
        assert!(unparenthesised_mixes("let h = (a * 3u) ^ (b * 5u);").is_empty());
    }

    #[test]
    fn includes_expand_exactly_once() {
        let source = expand("vg/raster_sw.wgsl");
        assert_eq!(source.matches("fn pcg(").count(), 1);
        assert!(!source.contains("#include"));
    }
}
