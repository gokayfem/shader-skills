//! What stands beside a circuit: the gantry over the start line, walls of tyres in front of
//! the barriers where a car is most likely to arrive, and the boards that count down the
//! distance to each slow corner.

use super::line::turning;
use super::{Land, Road};
use glam::{Mat4, Vec2, Vec3};
use solstice_asset::{Instance, Material, SceneAsset};
use solstice_builder::{build_virtual_mesh, BuildOptions, Mesh, Vertex};

/// A mesh being put together from flat pieces and turned shapes.
#[derive(Default)]
struct Shapes {
    mesh: Mesh,
}

impl Shapes {
    /// A flat piece with four corners, facing along `normal`.
    fn quad(&mut self, corners: [Vec3; 4], normal: Vec3) {
        let base = self.mesh.vertices.len() as u32;
        for position in corners {
            self.mesh.vertices.push(Vertex {
                position,
                normal,
                uv: Vec2::ZERO,
            });
        }
        let wound = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        let order: [u32; 6] = if wound.dot(normal) >= 0.0 {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        self.mesh.indices.extend(order.map(|index| base + index));
    }

    /// A box about a centre: `right`, `up` and `forward` reach from there to the middle of
    /// three of its faces.
    fn cuboid(&mut self, centre: Vec3, right: Vec3, up: Vec3, forward: Vec3) {
        for (out, a, b) in [
            (right, up, forward),
            (-right, up, forward),
            (up, right, forward),
            (-up, right, forward),
            (forward, right, up),
            (-forward, right, up),
        ] {
            let middle = centre + out;
            self.quad(
                [
                    middle - a - b,
                    middle + a - b,
                    middle + a + b,
                    middle - a + b,
                ],
                out.normalize_or_zero(),
            );
        }
    }

    /// A shape turned about an upright axis through `base`: `profile` is its outline, as how
    /// far out and how high, drawn so that the outside is on the right of the way it runs.
    fn lathe(&mut self, base: Vec3, profile: &[(f32, f32)], sides: usize) {
        for pair in profile.windows(2) {
            let ((r0, y0), (r1, y1)) = (pair[0], pair[1]);
            let slope = Vec2::new(y1 - y0, r0 - r1).normalize_or_zero();
            for side in 0..sides {
                let turn = |k: usize| k as f32 / sides as f32 * std::f32::consts::TAU;
                let point = |r: f32, y: f32, a: f32| base + Vec3::new(r * a.cos(), y, r * a.sin());
                let normal = |a: f32| Vec3::new(slope.x * a.cos(), slope.y, slope.x * a.sin());
                let (a, b) = (turn(side), turn(side + 1));
                let corners = [
                    point(r0, y0, a),
                    point(r1, y1, a),
                    point(r1, y1, b),
                    point(r0, y0, b),
                ];
                let normals = [normal(a), normal(a), normal(b), normal(b)];
                let start = self.mesh.vertices.len() as u32;
                for (position, normal) in corners.into_iter().zip(normals) {
                    self.mesh.vertices.push(Vertex {
                        position,
                        normal,
                        uv: Vec2::ZERO,
                    });
                }
                let wound = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                let order: [u32; 6] = if wound.dot(normal((a + b) * 0.5)) >= 0.0 {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                self.mesh.indices.extend(order.map(|index| start + index));
            }
        }
    }

    fn place(self, scene: &mut SceneAsset, material: u32) {
        if self.mesh.indices.is_empty() {
            return;
        }
        scene.instances.push(Instance {
            mesh: scene.meshes.len() as u32,
            material,
            transform: Mat4::IDENTITY,
        });
        scene
            .meshes
            .push(build_virtual_mesh(&self.mesh, &BuildOptions::default()));
    }
}

fn paint(scene: &mut SceneAsset, color: [f32; 3], roughness: f32, metallic: f32) -> u32 {
    scene.materials.push(Material {
        base_color: [color[0], color[1], color[2], 1.0],
        roughness,
        metallic,
        ..Default::default()
    });
    scene.materials.len() as u32 - 1
}

/// Which bars of a figure are lit for each digit: top, upper right, lower right, bottom, lower
/// left, upper left, middle.
const DIGITS: [[bool; 7]; 10] = [
    [true, true, true, true, true, true, false],
    [false, true, true, false, false, false, false],
    [true, true, false, true, true, false, true],
    [true, true, true, true, false, false, true],
    [false, true, true, false, false, true, true],
    [true, false, true, true, false, true, true],
    [true, false, true, true, true, true, true],
    [true, true, true, false, false, false, false],
    [true, true, true, true, true, true, true],
    [true, true, true, true, false, true, true],
];

/// Stands the circuit's furniture beside its road.
pub(super) fn furnish(scene: &mut SceneAsset, road: &Road, land: &Land) {
    let track = road.track;
    let samples = &track.samples;
    let count = samples.len();
    let graphite = paint(scene, [0.035, 0.037, 0.04], 0.5, 0.6);
    let white = paint(scene, [0.82, 0.82, 0.8], 0.6, 0.0);
    let red = paint(scene, [0.5, 0.035, 0.03], 0.6, 0.0);
    let black = paint(scene, [0.012, 0.012, 0.013], 0.85, 0.0);
    let lamp = paint(scene, [0.08, 0.005, 0.005], 0.3, 0.0);

    // ---- The gantry over the start line: a leg either side and a beam across, with the
    // start lights hung under its middle.
    let (centre, forward, left) = track.place(0.0);
    let level = (left * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
    let mut gantry = Shapes::default();
    let mut lamps = Shapes::default();
    let reach = [samples[0].half_left + 2.6, -(samples[0].half_right + 2.6)];
    let feet: Vec<Vec3> = reach
        .iter()
        .map(|&offset| {
            let p = centre + level * offset;
            Vec3::new(p.x, land.at(p.x, p.z), p.z)
        })
        .collect();
    let top = centre.y + 6.6;
    for foot in &feet {
        let height = top - foot.y;
        gantry.cuboid(
            *foot + Vec3::Y * height * 0.5,
            level * 0.16,
            Vec3::Y * height * 0.5,
            forward * 0.16,
        );
        // A foot plate.
        gantry.cuboid(
            *foot + Vec3::Y * 0.04,
            level * 0.4,
            Vec3::Y * 0.04,
            forward * 0.4,
        );
    }
    let span = (feet[0] - feet[1]) * Vec3::new(1.0, 0.0, 1.0);
    let middle = Vec3::new(
        (feet[0].x + feet[1].x) * 0.5,
        top,
        (feet[0].z + feet[1].z) * 0.5,
    );
    // Two rails with struts between them, as a truss.
    for high in [0.0_f32, -0.75] {
        gantry.cuboid(
            middle + Vec3::Y * high,
            span * 0.5 + level * 0.16,
            Vec3::Y * 0.07,
            forward * 0.07,
        );
    }
    let struts = (span.length() / 0.75) as usize;
    for k in 0..=struts {
        let along = k as f32 / struts as f32 - 0.5;
        let lean = if k % 2 == 0 { 1.0 } else { -1.0 };
        let (low, high) = (
            middle + span * (along - 0.25 / struts as f32 * lean) - Vec3::Y * 0.75,
            middle + span * (along + 0.25 / struts as f32 * lean),
        );
        let run = high - low;
        let across = run.cross(forward).normalize_or_zero();
        gantry.cuboid(
            (low + high) * 0.5,
            across * 0.035,
            run * 0.5,
            forward * 0.035,
        );
    }
    // The lights: five pods on a bar, facing the grid.
    let bar = middle - Vec3::Y * 1.15;
    gantry.cuboid(bar, level * 2.3, Vec3::Y * 0.26, forward * 0.1);
    for pod in 0..5 {
        let at = bar + level * (pod as f32 - 2.0) * 0.85;
        for row in [-0.11_f32, 0.11] {
            lamps.cuboid(
                at + Vec3::Y * row - forward * 0.11,
                level * 0.085,
                Vec3::Y * 0.085,
                forward * 0.02,
            );
        }
    }
    gantry.place(scene, graphite);
    lamps.place(scene, lamp);

    // ---- Walls of tyres in front of the barrier behind each gravel trap, in stacks of three,
    // painted red and white by turns.
    let mut stack = Shapes::default();
    let mut outline = Vec::new();
    for tyre in 0..3 {
        let base = tyre as f32 * 0.23;
        outline.extend([
            (0.30, base),
            (0.33, base + 0.04),
            (0.33, base + 0.19),
            (0.30, base + 0.23),
        ]);
    }
    // Over the top one's wall and down into its middle.
    outline.extend([(0.20, 0.69), (0.185, 0.62), (0.0, 0.62)]);
    stack.lathe(Vec3::ZERO, &outline, 14);
    let stack_mesh = scene.meshes.len() as u32;
    scene
        .meshes
        .push(build_virtual_mesh(&stack.mesh, &BuildOptions::default()));
    let mut stacks = 0;
    for (which, side) in [(0, 1.0_f32), (1, -1.0)] {
        let gravel = |i: usize| {
            let sample = &samples[i % count];
            let bands = if which == 0 {
                &sample.left_bands
            } else {
                &sample.right_bands
            };
            bands
                .iter()
                .filter(|band| band.0.name() == "gravel")
                .map(|band| band.1)
                .sum::<f32>()
        };
        let stands = |i: usize| {
            let sample = &samples[i % count];
            let flat = (sample.left * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
            sample.position + flat * side * (road.fence[which][i % count] - 0.6)
        };
        let mut owed = 0.0;
        for i in 0..count {
            if gravel(i) < 12.0 || gravel(i + 1) < 12.0 {
                owed = 0.0;
                continue;
            }
            let (a, b) = (stands(i), stands(i + 1));
            let length = a.distance(b);
            // One every 0.68 m along the wall, carried from one stretch to the next.
            let mut along = owed;
            while along < length {
                let p = a.lerp(b, along / length);
                let colour = if (stacks / 5) % 2 == 0 { red } else { white };
                scene.instances.push(Instance {
                    mesh: stack_mesh,
                    material: colour,
                    transform: Mat4::from_translation(Vec3::new(p.x, land.at(p.x, p.z), p.z))
                        * Mat4::from_rotation_y(stacks as f32 * 1.7),
                });
                stacks += 1;
                along += 0.68;
            }
            owed = along - length;
        }
    }

    // ---- Boards counting down to each slow corner: 150, 100 and 50 metres before it, on the
    // inside, where a driver looks for the place to brake.
    let flat: Vec<Vec2> = samples
        .iter()
        .map(|s| Vec2::new(s.position.x, s.position.z))
        .collect();
    let mut turn = turning(&flat);
    for _ in 0..6 {
        turn = (0..count)
            .map(|i| (turn[(i + count - 1) % count] + 2.0 * turn[i] + turn[(i + 1) % count]) * 0.25)
            .collect();
    }
    let slow = |i: usize| turn[i % count].abs() > 1.0 / 85.0;
    let step = track.length() / count as f32;
    let mut boards = 0;
    for entry in 0..count {
        // Where a slow corner begins, after a good stretch that was not one.
        if !slow(entry) || (1..=40).any(|k| slow(entry + count - k)) {
            continue;
        }
        let inside = turn[(entry + 4) % count].signum();
        for distance in [150u32, 100, 50] {
            let i = (entry + count - (distance as f32 / step).round() as usize) % count;
            // Only where the road there is all but straight: a board in a corner is no help.
            if turn[i].abs() > 1.0 / 220.0 {
                continue;
            }
            let sample = &samples[i];
            let across = (sample.left * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero() * inside;
            let ahead = (samples[(i + 1) % count].position - sample.position)
                .with_y(0.0)
                .normalize_or_zero();
            let (half, bands) = if inside > 0.0 {
                (sample.half_left, road.bands(i, true))
            } else {
                (sample.half_right, road.bands(i, false))
            };
            let foot = sample.position + across * (half + bands + 2.6);
            let foot = Vec3::new(foot.x, land.at(foot.x, foot.z), foot.z);
            // Facing the cars that come, and turned a little towards the road.
            let facing = (-ahead - across * 0.25).normalize_or_zero();
            // Each board is a mesh of its own: a mesh keeps its points to a 65,536th of its
            // size, and the figures stand only millimetres off the board.
            let (mut posts, mut faces, mut figures) =
                (Shapes::default(), Shapes::default(), Shapes::default());
            let wide = Vec3::Y.cross(facing).normalize_or_zero();
            let (width, height, lift) = (1.05, 0.72, 0.85);
            for leg in [-0.38_f32, 0.38] {
                posts.cuboid(
                    foot + wide * leg + Vec3::Y * (lift + height) * 0.5 - facing * 0.03,
                    wide * 0.025,
                    Vec3::Y * (lift + height) * 0.5,
                    facing * 0.025,
                );
            }
            let middle = foot + Vec3::Y * (lift + height * 0.5);
            faces.cuboid(
                middle,
                wide * width * 0.5,
                Vec3::Y * height * 0.5,
                facing * 0.012,
            );
            // The figures, in bars, as on a clock.
            let text = distance.to_string();
            let (tall, broad, thick, gap) = (0.44_f32, 0.24_f32, 0.058_f32, 0.085_f32);
            let total = text.len() as f32 * broad + (text.len() as f32 - 1.0) * gap;
            for (place, digit) in text.bytes().enumerate() {
                // `wide` is to the right of someone reading the board.
                let x = -total * 0.5 + broad * 0.5 + place as f32 * (broad + gap);
                let at = middle + wide * x + facing * 0.016;
                let lit = DIGITS[(digit - b'0') as usize];
                let (h, w) = (tall * 0.5, broad * 0.5);
                let bars = [
                    (0.0, h - thick * 0.5, w, thick * 0.5),
                    (w - thick * 0.5, h * 0.5, thick * 0.5, h * 0.5),
                    (w - thick * 0.5, -h * 0.5, thick * 0.5, h * 0.5),
                    (0.0, -(h - thick * 0.5), w, thick * 0.5),
                    (-(w - thick * 0.5), -h * 0.5, thick * 0.5, h * 0.5),
                    (-(w - thick * 0.5), h * 0.5, thick * 0.5, h * 0.5),
                    (0.0, 0.0, w, thick * 0.5),
                ];
                for (bar, &(bx, by, bw, bh)) in bars.iter().enumerate() {
                    if lit[bar] {
                        figures.cuboid(
                            at + wide * bx + Vec3::Y * by,
                            wide * bw,
                            Vec3::Y * bh,
                            facing * 0.004,
                        );
                    }
                }
            }
            posts.place(scene, graphite);
            faces.place(scene, white);
            figures.place(scene, black);
            boards += 1;
        }
    }
    println!("    {stacks} stacks of tyres and {boards} boards");
}
