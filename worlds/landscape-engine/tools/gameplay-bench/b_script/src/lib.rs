include!("../../common.rs");
use solstice_script::{Script, ScriptHost};
use solstice_world::{Game, Input, Schedule, Transform, World};
use std::cell::RefCell;
use std::rc::Rc;

const MOVER: &str = r#"
def on_begin(self):
    self.vars.count = 0

def on_tick(self, dt):
    self.vars.count += 1
    self.position += vec3(1, 0, 0) * dt * self.speed
"#;

const RUNAWAY: &str = r#"
def on_tick(self, dt):
    for i in range(1000000000):
        if i % 1000 == 0:
            log(i)
"#;

fn game(actors: usize, script: &str) -> (Game, Rc<RefCell<ScriptHost>>) {
    let host = Rc::new(RefCell::new(ScriptHost::new()));
    host.borrow_mut().load("script", script).unwrap();
    let mut schedule = Schedule::new();
    ScriptHost::install(&host, &mut schedule);
    let mut world = World::new();
    for _ in 0..actors {
        world.spawn((Transform::default(), Script::new("script").with_property("speed", 2.0)));
    }
    (Game::new(world, schedule), host)
}

pub fn bench() -> String {
    let mut out = String::new();
    for actors in [10, 100, 1000] {
        let (mut game, host) = game(actors, MOVER);
        let input = Input::new();
        game.step(&input);
        let steps = 30;
        let ms = best(3, || {
            for _ in 0..steps {
                game.step(&input);
            }
        }) / steps as f64;
        assert!(host.borrow_mut().take_errors().is_empty());
        out += &format!(
            "scripts: {actors} actors ticking: {ms:.3} ms a step ({:.2} us an actor)\n",
            ms * 1000.0 / actors as f64
        );
    }
    let (mut game, host) = game(1, RUNAWAY);
    let t = now_ms();
    game.step(&Input::new());
    let ms = now_ms() - t;
    let lines = host.borrow_mut().take_log().len();
    let errors = host.borrow_mut().take_errors().len();
    out += &format!("runaway: stopped after logging {lines} lines ({errors} error), in {ms:.1} ms\n");
    out
}
