include!("../../common.rs");
// Play mode's own cost each frame, on top of the scripts it runs: mapping input to actions,
// and working out where each actor is drawn and moving its instances.
use glam::Mat4;
use solstice_asset::GameFile;
use solstice_game::{Devices, Play, GAME_FILE};

const STILL: &str = "def on_poke(self):\n    pass\n";
const MOVER: &str = "def on_tick(self, dt):\n    self.yaw += dt\n";

fn play(actors: usize, script: &str) -> Play {
    let mut scene = String::new();
    for i in 0..actors {
        scene += &format!("actor a.actor {} 0 0 0 instances={}+1\n", i, i);
    }
    let file = |name: &str, text: String| GameFile { name: name.to_owned(), text };
    let files = vec![
        file(GAME_FILE, scene),
        file("a.actor", "name a\nscript a.star\n".to_owned()),
        file("a.star", script.to_owned()),
    ];
    Play::load(&files, |_| Some(Mat4::IDENTITY), None).unwrap()
}

pub fn bench() -> String {
    let mut out = String::new();
    let mut devices = Devices::new();
    devices.key("KeyW", true);
    devices.mouse_motion(3.0, 1.0);
    for (what, script) in [("still", STILL), ("turning", MOVER)] {
        for actors in [100, 1000] {
            let mut play = play(actors, script);
            let frames = 60;
            let mut moved = 0;
            let ms = best(3, || {
                for _ in 0..frames {
                    play.frame(1.0 / 60.0, &mut devices, |_, _| moved += 1);
                }
            }) / frames as f64;
            assert!(play.take_messages().is_empty());
            out += &format!(
                "play: {actors} actors {what}: {ms:.3} ms a frame ({:.2} us an actor), {} moves\n",
                ms * 1000.0 / actors as f64,
                moved / 3
            );
        }
    }
    out
}
