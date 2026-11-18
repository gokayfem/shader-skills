//! The camera and the per-frame constants derived from it.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};

/// A perspective camera with reversed depth and an infinite far plane.
///
/// Reversed Z spends floating-point precision where perspective needs it most, and
/// an infinite far plane removes one thing to tune.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub position: Vec3,
    /// Rotation around +Y, in radians. Zero looks along -Z.
    pub yaw: f32,
    /// Rotation above the horizon, in radians.
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 2.0, 10.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 55.0_f32.to_radians(),
            near: 0.05,
        }
    }
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(-sy * cp, sp, -cy * cp)
    }

    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, -sy)
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.position, self.forward(), Vec3::Y)
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(self.fov_y, aspect, self.near)
    }
}

/// How the sun and sky light the scene.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    /// Direction towards the sun.
    pub sun_direction: Vec3,
    /// Illuminance of the sun at the top of the atmosphere, in scene units.
    pub sun_illuminance: f32,
    pub sun_color: Vec3,
    pub sky_intensity: f32,
    /// Multiplier on indirect lighting. 1.0 is physically consistent.
    pub gi_intensity: f32,
    /// The sun's illuminance in full daylight, which exposure is held to, so that night stays
    /// dark rather than being brightened to grey. 0 means `sun_illuminance`.
    pub daylight_illuminance: f32,
    /// Radiance of the night sky's own faint glow, airglow and starlight, added to the sky. It
    /// shines by day too, lost in the sunlit sky.
    pub night_sky: f32,
    /// Radiance of the brightest stars, before cloud hides them. 0 by day.
    pub stars: f32,
    /// The celestial sphere: the north celestial pole, and the March equinox on the celestial
    /// equator, where right ascension starts. Together they place every star.
    pub star_pole: Vec3,
    pub star_equinox: Vec3,
    /// Where the sun and the moon stand in the sky, to draw them. The scene is lit from
    /// `sun_direction`, which by night is the moon. `None` draws the sun at `sun_direction`, and
    /// no moon.
    pub sun_disc: Option<Vec3>,
    pub moon_disc: Option<Vec3>,
    /// Radiance of the moon's sunlit highlands, seen face on.
    pub moon_radiance: f32,
}

impl Environment {
    /// The sun's illuminance in full daylight: `daylight_illuminance`, or the sun's own when
    /// that is unset.
    pub fn daylight(&self) -> f32 {
        if self.daylight_illuminance > 0.0 {
            self.daylight_illuminance
        } else {
            self.sun_illuminance
        }
    }

    /// How far it is day: 1 under a sun well up, 0 by night and from when the sun reaches the
    /// horizon, so that dusk is as dark as night makes it.
    pub fn day(&self) -> f32 {
        let step = |low: f32, high: f32, x: f32| {
            let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let daylight = self.daylight();
        if daylight <= 0.0 {
            return 0.0;
        }
        // A tenth of a radian or so up, the sun lights the ground as it does all day. The
        // moon, which lights the night from `sun_direction` too, gives a 2500th of the sun.
        step(0.0, 0.2, self.sun_direction.y) * step(0.01, 0.5, self.sun_illuminance / daylight)
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(0.45, 0.35, 0.3).normalize(),
            sun_illuminance: 10.0,
            sun_color: Vec3::ONE,
            sky_intensity: 1.0,
            gi_intensity: 1.0,
            daylight_illuminance: 0.0,
            night_sky: 0.0,
            stars: 0.0,
            star_pole: Vec3::Y,
            star_equinox: Vec3::X,
            sun_disc: None,
            moon_disc: None,
            moon_radiance: 0.0,
        }
    }
}

/// What the renderer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum DebugView {
    #[default]
    Lit = 0,
    Clusters = 1,
    Triangles = 2,
    RasterMode = 3,
    Instances = 4,
    Albedo = 5,
    Normals = 6,
    DiffuseGi = 7,
    Reflections = 8,
    Voxels = 9,
    /// The sea's normals, where it is drawn.
    Ocean = 10,
    /// How wet each surface is: dark when dry, blue when soaked.
    Wetness = 11,
    /// How far each pixel moved since the last frame: grey where nothing moved, more red as it
    /// moved right on screen and more green as it moved up.
    Motion = 12,
    /// How many layers of particles cover each pixel: dark red for one, through yellow to white.
    Particles = 13,
    /// The light each particle receives, as a white particle would scatter it, without haze.
    ParticleLight = 14,
    /// The wind near the ground, where something sways in it: brighter where it blows harder,
    /// its hue the way it blows (`docs/weather-effects.md`). Dark in a scene with nothing
    /// that sways.
    Wind = 15,
}

impl DebugView {
    /// Parses a mode name, as used by the viewer's command line and UI.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "lit" => Self::Lit,
            "clusters" => Self::Clusters,
            "triangles" => Self::Triangles,
            "raster" => Self::RasterMode,
            "instances" => Self::Instances,
            "albedo" => Self::Albedo,
            "normals" => Self::Normals,
            "gi" => Self::DiffuseGi,
            "reflections" => Self::Reflections,
            "voxels" => Self::Voxels,
            "ocean" => Self::Ocean,
            "wetness" => Self::Wetness,
            "motion" => Self::Motion,
            "particles" => Self::Particles,
            "particle-light" => Self::ParticleLight,
            "wind" => Self::Wind,
            _ => return None,
        })
    }
}

/// The sea level the shaders are given when there is no sea: far below anything, so that nothing
/// is under water or wet from it.
pub const NO_SEA: f32 = -1.0e30;

/// How many trucks that stand about can each be in a colour of their own.
pub const RESPRAYS: usize = 8;

/// A run of instances in a colour of their own: a truck that stands about, which nobody
/// drives. Its paint is `paint` wherever it was made `was`, as a [`Coat`]'s is. Twin of
/// `Respray` in `common/frame.wgsl`; none of them when `count` is 0.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Respray {
    pub paint: Vec3,
    pub first: u32,
    pub was: Vec3,
    pub count: u32,
}

/// Twin of `Frame` in `common/frame.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameUniform {
    pub view_proj: Mat4,
    pub inv_view_proj: Mat4,
    pub unjittered_view_proj: Mat4,
    pub prev_view_proj: Mat4,
    pub view: Mat4,
    pub camera_position: Vec3,
    pub frame_index: u32,
    pub viewport: Vec2,
    pub inv_viewport: Vec2,
    pub jitter_ndc: Vec2,
    pub near: f32,
    pub debug_mode: u32,
    pub sun_direction: Vec3,
    pub sun_illuminance: f32,
    pub sun_color: Vec3,
    pub sky_intensity: f32,
    pub light_count: u32,
    pub time: f32,
    pub gi_intensity: f32,
    pub temporal_accumulation: u32,
    /// Scales texture gradients: internal over output resolution, so that upscaling
    /// keeps the detail of the output resolution, times a detail bias for temporal filtering.
    pub mip_scale: f32,
    pub night_sky: f32,
    pub stars: f32,
    /// Radiance of the moon's sunlit highlands; 0 draws no moon.
    pub moon_radiance: f32,
    pub star_pole: Vec3,
    /// The sun's illuminance at noon, which night is measured against.
    pub daylight_illuminance: f32,
    pub star_equinox: Vec3,
    /// Seconds since the previous frame, which temporal filters scale their blend by.
    pub frame_seconds: f32,
    /// Where the sun and moon are drawn: the true sun, whatever lights the scene.
    pub sun_disc: Vec3,
    /// The calm sea's level; [`NO_SEA`] when there is none.
    pub sea_level: f32,
    pub moon_disc: Vec3,
    pub _pad2: f32,
    /// The lamps of whatever is driven: see [`Lamp`]. No lamp when the range is 0.
    pub lamp_position: Vec3,
    pub lamp_range: f32,
    pub lamp_direction: Vec3,
    pub lamp_spread: f32,
    pub lamp_intensity: Vec3,
    pub _pad3: f32,
    /// The coat of whatever is driven: see [`Coat`]. Nothing wears one when `coat_count` is 0.
    /// The rows that take a place in the world to the car's own space.
    pub coat_x: Vec4,
    pub coat_y: Vec4,
    pub coat_z: Vec4,
    pub coat_paint: Vec3,
    pub coat_rust: f32,
    pub coat_soil: Vec3,
    pub coat_mud: f32,
    pub coat_was: Vec3,
    pub coat_dust: f32,
    pub coat_first: u32,
    pub coat_count: u32,
    pub coat_seed: u32,
    pub coat_wet: f32,
    /// Where its axles are along it, half its track, and its wheels' radius.
    pub coat_axles: Vec4,
    /// Trucks that stand about, each in its own colour.
    pub resprays: [Respray; RESPRAYS],
    /// The ruts pressed into the ground (`crate::ruts::RutMap::window`): the middle of the
    /// square of them that is held, half its width, 0 when nothing is held, and one over
    /// the width of ground the map wraps round in.
    pub rut_window: Vec4,
    /// The colour of churned soil, and the width of a cell of that map, in metres.
    pub rut_soil: Vec3,
    pub rut_cell: f32,
}

/// What whatever is driven wears: the colour of its paint, and what the country has put on
/// it. All of it is worked out where the car is drawn, from where each point of it is in
/// the car's own space, so none of it is a texture and any of it can change from one frame
/// to the next.
///
/// - **Paint.** A material marked `Material::PAINT` is given `paint` in place of the colour
///   it was made in, `was`, keeping its shade: what in its texture is some other colour
///   (a stripe, a badge) is left alone.
/// - **Rust**, from 0 to 1, on the paint: low on the panels and round the wheel arches
///   first, in blotches that spread and run.
/// - **Dust**, from 0 to 1: a film over everything out in the weather, thickest on what
///   faces up and towards the back.
/// - **Mud**, from 0 to 1: caked from the ground up, thrown up behind each wheel, spattered
///   above that; `wet` says how fresh it is, from 0, dried, to 1.
///
/// A material marked `Material::INSIDE` takes no dust or mud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coat {
    /// Where the car is: its own space is metres from the ground under its middle, x to
    /// its right, y up, z to its back.
    pub car: Mat4,
    pub paint: Vec3,
    /// The colour its paint was made in, to tell paint from what is painted on it.
    pub was: Vec3,
    pub rust: f32,
    pub dust: f32,
    pub mud: f32,
    pub wet: f32,
    /// The colour of the ground it has been driven on: its mud's, and its dust's.
    pub soil: Vec3,
    /// Makes one car's rust and mud its own.
    pub seed: u32,
    /// Where its front and rear axles are along z, half its track, and its wheels' radius.
    pub axles: [f32; 4],
}

/// The lamps of whatever is driven, as one beam: a car's headlights. It moves with the car,
/// and casts no shadows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub position: Vec3,
    /// The way it points: a unit vector.
    pub direction: Vec3,
    /// Half the beam's spread, in radians: it is brightest along its middle and gone here.
    pub half_angle: f32,
    /// How far it reaches, in metres.
    pub range: f32,
    /// Linear RGB intensity, in candela, as a scene's lights are.
    pub intensity: Vec3,
}

/// Sub-pixel offset for frame `index`, from the Halton (2, 3) sequence, in `-0.5..0.5`.
pub fn jitter_offset(index: u32) -> Vec2 {
    Vec2::new(halton(index % 8 + 1, 2), halton(index % 8 + 1, 3)) - Vec2::splat(0.5)
}

fn halton(mut index: u32, base: u32) -> f32 {
    let (mut fraction, mut result) = (1.0, 0.0);
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
}

/// Adds a constant NDC offset to a projection matrix without changing depth.
pub fn jitter_projection(projection: Mat4, jitter_ndc: Vec2) -> Mat4 {
    let mut jittered = projection;
    // With w = -z_view, subtracting from the Z column shifts NDC by a constant amount.
    jittered.z_axis.x -= jitter_ndc.x;
    jittered.z_axis.y -= jitter_ndc.y;
    jittered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WGSL `Frame` must match `FrameUniform` byte for byte, or every value after the first
    /// difference is read from the wrong place. Naga lays out the WGSL; Rust reports its own.
    #[test]
    fn the_uniform_lays_out_as_its_shader_twin() {
        let source = include_str!("../shaders/common/frame.wgsl");
        let module = naga::front::wgsl::parse_str(source).expect("frame.wgsl parses");
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).unwrap();
        let (handle, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("Frame"))
            .expect("a Frame struct");
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("Frame is a struct");
        };
        assert_eq!(*span as usize, std::mem::size_of::<FrameUniform>(), "size");
        assert_eq!(layouter[handle].size, *span);
        macro_rules! offsets {
            ($($field:ident),*) => {
                vec![$((stringify!($field), std::mem::offset_of!(FrameUniform, $field))),*]
            };
        }
        let rust = offsets!(
            view_proj,
            inv_view_proj,
            unjittered_view_proj,
            prev_view_proj,
            view,
            camera_position,
            frame_index,
            viewport,
            inv_viewport,
            jitter_ndc,
            near,
            debug_mode,
            sun_direction,
            sun_illuminance,
            sun_color,
            sky_intensity,
            light_count,
            time,
            gi_intensity,
            temporal_accumulation,
            mip_scale,
            night_sky,
            stars,
            star_pole,
            daylight_illuminance,
            star_equinox,
            moon_radiance,
            frame_seconds,
            sun_disc,
            sea_level,
            moon_disc,
            coat_axles,
            resprays,
            rut_window,
            rut_soil,
            rut_cell
        );
        for (name, offset) in rust {
            let member = members
                .iter()
                .find(|m| m.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name} is missing from the WGSL"));
            assert_eq!(member.offset as usize, offset, "{name}");
        }
    }

    /// Exposure reaches further into the dark by day than by night, so what counts as day decides
    /// whether a night is brightened to grey. Only a sun that is up counts: not the moon, which
    /// lights the scene from `sun_direction` too, nor lamps under no sun at all.
    #[test]
    fn it_is_day_only_under_a_sun_that_is_up() {
        let noon = Environment::default();
        assert_eq!(noon.day(), 1.0);
        let at = |height: f32| Environment {
            sun_direction: Vec3::new(1.0 - height * height, height, 0.0).normalize(),
            ..Environment::default()
        };
        assert_eq!(at(0.0).day(), 0.0, "the sun on the horizon");
        assert_eq!(at(-0.2).day(), 0.0, "the sun below it");
        let mut previous = 0.0;
        for step in 0..=30 {
            let day = at(step as f32 * 0.01).day();
            assert!(day >= previous, "it grew less day as the sun rose");
            previous = day;
        }
        assert_eq!(previous, 1.0, "a sun 17 degrees up");
        let moonlit = Environment {
            sun_illuminance: 10.0 / 2500.0,
            daylight_illuminance: 10.0,
            ..noon
        };
        assert_eq!(moonlit.day(), 0.0, "a moon high in the sky");
        let lamplit = Environment {
            sun_illuminance: 0.0,
            daylight_illuminance: 1500.0,
            ..noon
        };
        assert_eq!(lamplit.day(), 0.0, "no sun at all");
        let nothing = Environment {
            sun_illuminance: 0.0,
            ..noon
        };
        assert_eq!(
            nothing.day(),
            0.0,
            "no sun and no daylight to compare it with"
        );
    }

    #[test]
    fn forward_matches_the_view_matrix() {
        let camera = Camera {
            yaw: 0.7,
            pitch: -0.3,
            position: Vec3::new(1.0, 2.0, 3.0),
            ..Default::default()
        };
        let ahead = camera
            .view()
            .transform_point3(camera.position + camera.forward());
        assert!(ahead.distance(Vec3::new(0.0, 0.0, -1.0)) < 1.0e-5);
    }

    #[test]
    fn projection_maps_near_plane_to_one_and_far_towards_zero() {
        let camera = Camera::default();
        let projection = camera.projection(1.5);
        let near = projection.project_point3(Vec3::new(0.0, 0.0, -camera.near));
        let far = projection.project_point3(Vec3::new(0.0, 0.0, -1.0e6));
        assert!((near.z - 1.0).abs() < 1.0e-5);
        assert!(far.z < 1.0e-5);
    }

    #[test]
    fn jitter_shifts_every_depth_by_the_same_amount() {
        let projection = Camera::default().projection(1.5);
        let offset = Vec2::new(0.01, -0.02);
        let jittered = jitter_projection(projection, offset);
        for z in [-0.5, -5.0, -500.0] {
            let p = Vec3::new(0.3, 0.2, z);
            let delta = jittered.project_point3(p) - projection.project_point3(p);
            assert!((delta.x - offset.x).abs() < 1.0e-5);
            assert!((delta.y - offset.y).abs() < 1.0e-5);
            assert!(delta.z.abs() < 1.0e-6);
        }
    }

    #[test]
    fn jitter_sequence_stays_within_half_a_pixel_and_averages_near_zero() {
        let mut sum = Vec2::ZERO;
        for i in 0..8 {
            let j = jitter_offset(i);
            assert!(j.x.abs() <= 0.5 && j.y.abs() <= 0.5);
            sum += j;
        }
        assert!((sum / 8.0).length() < 0.15);
    }
}
