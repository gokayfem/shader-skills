//! Play mode in a viewer: a scene's game, driving the renderer's instances and camera.
//!
//! Built only with the `gameplay` feature, so the plain web viewer carries none of it.

use solstice_asset::{Collision, GameFile};
use solstice_game::animation::AnimationAssets;
use solstice_game::rut::{self, Ruts};
pub use solstice_game::Message;
use solstice_game::{Devices, Play};
use solstice_render::{Camera, Renderer, RutSource};

// The map of ruts is copied to the renderer a square at a time, as it is kept: both must agree
// on what a square is.
const _: () = assert!(
    rut::TILE == solstice_render::ruts::TILE as usize
        && rut::LEVELS == solstice_render::ruts::LEVELS as usize
);

/// The map of ruts, as the renderer asks for it.
struct Kept<'a>(&'a Ruts);

impl RutSource for Kept<'_> {
    fn cell(&self) -> f32 {
        rut::CELL
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn version(&self, tile: (i32, i32)) -> u32 {
        self.0.version(tile)
    }

    fn levels(&self, tile: (i32, i32), layer: usize) -> Option<Vec<Vec<u8>>> {
        let mut levels = vec![self.0.layer(tile, layer)?.to_vec()];
        levels.extend(self.0.smaller(tile, layer)?);
        Some(levels)
    }
}

/// Brings the ruts the renderer draws up to date with the ones the game keeps (docs/ruts.md),
/// round `centre`: the car, or wherever is looked at. Nothing is done on ground nobody has
/// driven on.
fn draw_ruts(renderer: &mut Renderer, play: &Play, centre: glam::Vec3) {
    play.ruts(|ruts| {
        // Nobody else keeps a list of what changed: each square's own count says.
        ruts.take_changed();
        renderer.sync_ruts(glam::Vec2::new(centre.x, centre.z), &Kept(ruts));
    });
}

/// A scene's game, running in a viewer.
pub struct PlayMode {
    pub play: Play,
    /// What the input devices are doing, filled in from the window's events.
    pub devices: Devices,
}

impl PlayMode {
    /// Starts the game in a scene's gameplay files, making every instance an actor draws with
    /// movable, and giving each animated actor its skeleton and clips. Returns every mistake
    /// found in the files, each with its file and line.
    pub fn start(
        renderer: &mut Renderer,
        files: &[GameFile],
        collision: Option<&Collision>,
        animation: &AnimationAssets<'_>,
    ) -> Result<Self, Vec<String>> {
        let mut play = Play::load(files, |id| renderer.instance_transform(id), collision)?;
        play.attach_animation(files, animation)?;
        for id in play.instances() {
            renderer.set_instance_movable(id, true);
        }
        Ok(Self {
            play,
            devices: Devices::new(),
        })
    }

    /// Runs a frame `seconds` long: moves the instances that moved, poses the characters that
    /// changed, and puts the camera where the player's eye is, if there is a player. Returns
    /// what scripts logged and every error.
    pub fn frame(
        &mut self,
        renderer: &mut Renderer,
        camera: &mut Camera,
        seconds: f32,
    ) -> Vec<Message> {
        self.play.set_viewer(camera.position);
        let view = self
            .play
            .frame(f64::from(seconds), &mut self.devices, |id, world| {
                renderer.move_instance(id, world);
            });
        for (pose, bones) in self.play.take_poses() {
            renderer.set_pose(pose, &bones);
        }
        if let Some(view) = view {
            camera.position = view.position;
            camera.yaw = view.yaw;
            camera.pitch = view.pitch;
        }
        // What the car's tyres have pressed into soft ground is drawn round the car.
        let driven = self.play.hud().filter(|hud| !hud.afoot);
        let looked_at = driven.map_or(camera.position, |hud| hud.origin);
        draw_ruts(renderer, &self.play, looked_at);
        self.play.take_messages()
    }
}
