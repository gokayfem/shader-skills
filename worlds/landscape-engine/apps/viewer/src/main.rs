//! Native viewer: an interactive window, or a headless capture for automated checks.
//!
//! ```text
//! viewer [SCENE] [--capture out.png] [--frames N] [--size WxH]
//!        [--camera x,y,z,yaw,pitch] [--debug MODE] [--sun x,y,z] [--unpack-textures]
//!        [--weather NAME|FILE [--weathered SECONDS]] [--strike-at FRAME]
//!        [--limits webgpu]
//! viewer [SCENE] --flyover PATH.txt --out DIR [--stability] [--no-taa] [--size WxH] [--quality Q] [--low-power]
//! viewer [SCENE] --play [--fly] [--scripts DIR] [--capture out.png --frames N
//!        [--press ACTION@FRAME]... [--hold KEY@FROM-TO]... [--mouse PIXELS@FRAME]...]
//! viewer [SCENE] --game DIR [the same as with --play]
//! viewer [SCENE] --instances-near x,y,z,radius
//! ```
//!
//! `--play` runs the scene's game, if it has one: its actors move, the camera is the player's,
//! and keys and the mouse are its input. Its scripts are reloaded from `--scripts` (by default
//! `scenes`, where scene text files live) when they change on disk. **V** leaves the player
//! standing and flies the camera from where it is, and comes back; `--fly` starts that way. The
//! game goes on meanwhile, and still hears the keys: a door opens to the camera flown up to it.
//!
//! `--game DIR` plays the game in a folder in place of the one baked into the scene, if any:
//! its `scene.game`, and the actor files, scripts and input actions that names. So a scene
//! baked without a game, or one too big to bake again, is given one, or a new one, without a
//! bake. Its `actor` lines name the scene's instances themselves (`instances=FIRST+COUNT`),
//! which `--instances-near` lists: each instance placed within a radius of a point, with its
//! mesh, material, place and yaw.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    native::run()
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use glam::Vec3;
    use solstice_asset::paged::PagedScene;
    use solstice_asset::scene::Viewpoint;
    use solstice_asset::LoadedScene;
    use solstice_render::{
        Camera, DebugView, Effect, Environment, Gpu, Ocean, Pond, Quality, Renderer,
        RendererSettings, StreamRequest, StreamingBudget, TimeOfDay, Weather,
    };
    use solstice_viewer::bob::Bobbing;
    use solstice_viewer::play::PlayMode;
    use solstice_viewer::{
        default_environment, initial_camera, parse_time, CameraInput, DayClock, FlyCamera,
    };
    use std::collections::HashSet;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Instant;
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::keyboard::{KeyCode, PhysicalKey};
    use winit::window::{CursorGrabMode, Window, WindowId};

    const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// Reads pages from a scene file on background threads.
    struct PageReader {
        path: Arc<PathBuf>,
        finished_sender: mpsc::Sender<(u32, Option<Vec<u8>>)>,
        finished: mpsc::Receiver<(u32, Option<Vec<u8>>)>,
        running: u32,
    }

    impl PageReader {
        /// Pages to read at once: enough to use the disk, few enough to stay out of the way.
        const MAX_FETCHING: u32 = 256;

        fn new(path: &Path) -> Self {
            let (finished_sender, finished) = mpsc::channel();
            Self {
                path: Arc::new(path.to_owned()),
                finished_sender,
                finished,
                running: 0,
            }
        }

        /// Hands over what has been read, and starts reading what's now wanted. Call per frame.
        fn pump(&mut self, renderer: &mut Renderer) {
            while let Ok((id, data)) = self.finished.try_recv() {
                self.running -= 1;
                match data {
                    Some(data) => {
                        renderer.deliver_fetched(id, data);
                    }
                    None => renderer.request_failed(id),
                }
            }
            for request in renderer.stream_requests(Self::MAX_FETCHING) {
                self.running += 1;
                let path = self.path.clone();
                let sender = self.finished_sender.clone();
                std::thread::spawn(move || {
                    let data = read_range(&path, &request).ok();
                    let _ = sender.send((request.id, data));
                });
            }
        }

        fn is_idle(&self) -> bool {
            self.running == 0
        }
    }

    fn read_range(
        path: &Path,
        request: &StreamRequest,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut file = std::fs::File::open(path)?;
        // A scene joined with the file it shares with others is two files, whose bytes are
        // numbered straight on from one to the other: past the end of the scene's is the
        // shared one (`read_source`).
        let length = file.metadata()?.len();
        if request.start >= length {
            file = std::fs::File::open(path.with_file_name(solstice_viewer::shared::FILE))?;
            file.seek(SeekFrom::Start(request.start - length))?;
        } else {
            file.seek(SeekFrom::Start(request.start))?;
        }
        let mut bytes = vec![0; (request.end - request.start) as usize];
        file.read_exact(&mut bytes)?;
        if request.deflated {
            bytes = solstice_asset::file::inflate(&bytes)?;
        }
        Ok(bytes)
    }

    /// Reads just the part of a scene file that stays resident.
    fn read_manifest(path: &Path) -> Result<PagedScene, Box<dyn std::error::Error>> {
        let mut file = std::fs::File::open(path)?;
        let mut head = [0; solstice_asset::file::HEADER_BYTES];
        file.read_exact(&mut head)?;
        let header = solstice_asset::file::decode_header(&head)?;
        file.seek(SeekFrom::Start(header.manifest_offset))?;
        let mut packed = vec![0; header.manifest_bytes as usize];
        file.read_exact(&mut packed)?;
        let manifest = solstice_asset::file::inflate(&packed)?;
        let scene = solstice_asset::file::decode_manifest(
            &manifest,
            header.manifest_offset + header.manifest_bytes,
        )?;
        if scene.external.is_some() {
            return Err("this is the file made for browsers: open the .solstice file".into());
        }
        Ok(scene)
    }

    /// A scene as this run holds it: all in memory, or read from disk as wanted.
    enum Source {
        Loaded(LoadedScene),
        Streamed(PagedScene),
    }

    impl Source {
        fn viewpoint(&self) -> Option<Viewpoint> {
            match self {
                Self::Loaded(loaded) => loaded.scene.viewpoint,
                Self::Streamed(paged) => paged.viewpoint,
            }
        }

        /// The scene's collision, whole, read from its file if it wasn't all loaded.
        fn collision(
            &self,
            path: &Path,
        ) -> Result<Option<solstice_asset::Collision>, Box<dyn std::error::Error>> {
            let paged = match self {
                Self::Loaded(loaded) => return Ok(loaded.collision.clone()),
                Self::Streamed(paged) => paged,
            };
            let Some(index) = &paged.collision else {
                return Ok(None);
            };
            let mut file = std::fs::File::open(path)?;
            let mut pieces = Vec::new();
            for ((offset, stored), raw) in index.ranges().into_iter().zip(index.raw_sizes()) {
                file.seek(SeekFrom::Start(paged.data_offset + offset))?;
                let mut packed = vec![0; stored as usize];
                file.read_exact(&mut packed)?;
                pieces.push(solstice_asset::file::inflate_limited(&packed, raw)?);
            }
            Ok(Some(index.decode(&pieces)?))
        }

        /// Prints the instances placed within a radius of a point, `near` being x, y, z and the
        /// radius: what a scene's game needs to say which instances an actor moves.
        fn list_instances(&self, near: [f32; 4]) {
            let paged = match self {
                Self::Loaded(loaded) => &loaded.scene,
                Self::Streamed(paged) => paged,
            };
            let [x, y, z, radius] = near;
            println!("instance  mesh  material  x y z  yaw");
            for (id, instance) in paged.instances.iter().enumerate() {
                let (_, rotation, at) = instance.transform.to_scale_rotation_translation();
                if at.distance(Vec3::new(x, y, z)) > radius {
                    continue;
                }
                let yaw = rotation.to_euler(glam::EulerRot::YXZ).0.to_degrees();
                println!(
                    "{id}  {}  {}  {:.3} {:.3} {:.3}  {yaw:.1}",
                    instance.mesh, instance.material, at.x, at.y, at.z
                );
            }
        }

        fn gameplay(&self) -> &[solstice_asset::GameFile] {
            match self {
                Self::Loaded(loaded) => &loaded.scene.gameplay,
                Self::Streamed(paged) => &paged.gameplay,
            }
        }

        fn animation(&self) -> solstice_game::animation::AnimationAssets<'_> {
            let paged = match self {
                Self::Loaded(loaded) => &loaded.scene,
                Self::Streamed(paged) => paged,
            };
            solstice_game::animation::AnimationAssets {
                skeletons: &paged.skeletons,
                clips: &paged.clips,
                poses: &paged.poses,
                instance_poses: &paged.instance_poses,
            }
        }

        fn renderer(
            &self,
            gpu: &Gpu,
            format: wgpu::TextureFormat,
            options: &Options,
        ) -> (Renderer, Option<PageReader>) {
            let (mut renderer, reader) = match self {
                Self::Loaded(loaded) => {
                    (Renderer::new(gpu, loaded, format, options.settings), None)
                }
                Self::Streamed(paged) => (
                    Renderer::streaming(gpu, paged, format, options.settings, options.budget),
                    Some(PageReader::new(&options.scene)),
                ),
            };
            renderer.set_weather(options.weather, 0.0);
            renderer.advance_weather(options.weathered);
            if options.ocean.is_some() {
                renderer.set_ocean(options.ocean, 0.0);
            }
            if let Some(level) = options.sea_level {
                renderer.set_sea_level(level);
            }
            // A scene's own ponds stand unless others are asked for.
            if !options.ponds.is_empty() {
                renderer.set_ponds(&options.ponds);
            }
            spawn_effects(&mut renderer, options);
            (renderer, reader)
        }
    }

    struct Options {
        /// Load pages from disk as they're wanted, rather than all at the start.
        stream: bool,
        /// GPU memory for the scene when streaming.
        budget: StreamingBudget,
        scene: PathBuf,
        capture: Option<PathBuf>,
        frames: u32,
        size: (u32, u32),
        /// Overrides the scene's own viewpoint.
        camera: Option<Camera>,
        environment: Environment,
        settings: RendererSettings,
        power: wgpu::PowerPreference,
        /// Acts as if the GPU had no block compression, to test that path.
        unpack_textures: bool,
        /// Asks for no more than these limits of the GPU, to see what a small one is given:
        /// `--limits webgpu`, the WebGPU defaults, which many phones are given.
        limits: Option<wgpu::Limits>,
        /// Metres per frame the capture camera slides sideways, for moving-view benchmarks.
        capture_motion: f32,
        /// A camera path to fly, with results written to `flyover_out`.
        flyover: Option<PathBuf>,
        flyover_out: PathBuf,
        /// Measure frame-to-frame change during a flyover. Slows the flight, so not for timing.
        stability: bool,
        /// The weather to start with: a built-in state's name, or a weather file.
        weather: Weather,
        /// Seconds the weather has already gone on for, as if they had passed: how long it has
        /// rained, for one.
        weathered: f32,
        /// A frame of a capture on which lightning strikes, whatever the weather.
        strike_at: Option<u32>,
        /// When set, the sun and moon follow the time of day instead of `environment`.
        clock: Option<DayClock>,
        /// An open sea, by name or file, and the level of its calm surface. Scenes with a sea of
        /// their own bring both.
        ocean: Option<Ocean>,
        sea_level: Option<f32>,
        /// Still water to lay over the scene: each a rectangle and the water's height.
        ponds: Vec<Pond>,
        /// Particle effects to run, by name or file, and where.
        effects: Vec<(Effect, Vec3)>,
        /// Instances to rock as if afloat: those centred within a radius of a point across the
        /// ground, as x, z, radius, the height they rock about, and how far they rock.
        bob: Option<[f32; 5]>,
        /// Run the scene's game.
        play: bool,
        /// Start the game flying the camera, with whoever is played left standing.
        fly: bool,
        /// A folder holding the game to run, in place of the scene's own.
        game: Option<PathBuf>,
        /// List the instances placed within a radius of a point, as x, y, z, radius, and stop.
        instances_near: Option<[f32; 4]>,
        /// Where the scene's scripts are on disk, by the names the scene gives them, to reload
        /// them when they change.
        scripts: PathBuf,
        /// Actions to press in a capture, and the frame to press each in.
        presses: Vec<(String, u32)>,
        /// Keys to hold in a capture, by their `KeyboardEvent.code`, and from which frame to
        /// which.
        holds: Vec<(String, u32, u32)>,
        /// The mouse moved in a capture, by pixels across, at a frame.
        mouse_moves: Vec<(f32, u32)>,
    }

    /// Starts the effects the options name.
    fn spawn_effects(renderer: &mut Renderer, options: &Options) {
        for (effect, at) in &options.effects {
            renderer
                .particles()
                .spawn(effect, glam::Mat4::from_translation(*at));
        }
    }

    /// Makes the instances `--bob` names movable, ready to rock.
    fn bobbing(options: &Options, renderer: &mut Renderer) -> Option<Bobbing> {
        let [x, z, radius, height, amplitude] = options.bob?;
        let bobbing = Bobbing::around(renderer, [x, z], radius, height, amplitude);
        log::info!("{} instances bob around ({x}, {z})", bobbing.len());
        Some(bobbing)
    }

    /// Starts the scene's game, if `--play` asks and the scene has one, logging every mistake
    /// in its files if it can't start.
    fn start_play(options: &Options, scene: &Source, renderer: &mut Renderer) -> Option<PlayMode> {
        if !options.play {
            return None;
        }
        let side;
        let files = match &options.game {
            None => scene.gameplay(),
            Some(folder) => {
                let mut files = Vec::new();
                if let Err(error) = read_game_files(folder, "", &mut files) {
                    log::error!("--game {}: {error}", folder.display());
                    return None;
                }
                side = files;
                &side
            }
        };
        if files.is_empty() {
            log::warn!("--play: this scene has no gameplay, so there is nothing to play");
            return None;
        }
        let collision = match scene.collision(&options.scene) {
            Ok(collision) => collision,
            Err(error) => {
                log::error!("the scene's collision can't be read: {error}");
                None
            }
        };
        if collision.is_none() {
            log::warn!("this scene has no collision: rebake it to walk on it");
        }
        match PlayMode::start(renderer, files, collision.as_ref(), &scene.animation()) {
            Ok(mut play) => {
                log::info!(
                    "playing: {} instances move, scripts {}",
                    play.play.instances().count(),
                    play.play.scripts().join(", ")
                );
                play.play.set_flying(options.fly);
                Some(play)
            }
            Err(errors) => {
                for error in errors {
                    log::error!("{error}");
                }
                None
            }
        }
    }

    /// Reads every text file in a folder, and the folders in it, as gameplay files named as a
    /// scene's are: from the folder, with `/` between folders.
    fn read_game_files(
        folder: &Path,
        prefix: &str,
        files: &mut Vec<solstice_asset::GameFile>,
    ) -> std::io::Result<()> {
        let mut entries = std::fs::read_dir(folder)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type()?.is_dir() {
                read_game_files(&entry.path(), &format!("{name}/"), files)?;
            } else if let Ok(text) = std::fs::read_to_string(entry.path()) {
                files.push(solstice_asset::GameFile { name, text });
            }
        }
        Ok(())
    }

    /// Logs what a game's scripts logged, and its errors.
    fn log_messages(messages: Vec<solstice_viewer::play::Message>) {
        for message in messages {
            if message.error {
                log::error!("{}", message.text);
            } else {
                log::info!("{}", message.text);
            }
        }
    }

    /// Reloads a game's scripts when they change on disk.
    struct ScriptWatch {
        /// Each script's name in the game, its file, and when that last changed.
        files: Vec<(String, PathBuf, Option<std::time::SystemTime>)>,
        since_check: f32,
    }

    impl ScriptWatch {
        /// How often to look, in seconds.
        const INTERVAL: f32 = 0.5;

        fn new(play: &PlayMode, folder: &Path) -> Self {
            let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
            let files = play
                .play
                .scripts()
                .iter()
                .map(|name| {
                    let path = folder.join(name);
                    let time = modified(&path);
                    if time.is_none() {
                        log::warn!("{} is not there to watch", path.display());
                    }
                    (name.clone(), path, time)
                })
                .collect();
            Self {
                files,
                since_check: 0.0,
            }
        }

        fn check(&mut self, play: &mut PlayMode, seconds: f32) {
            self.since_check += seconds;
            if self.since_check < Self::INTERVAL {
                return;
            }
            self.since_check = 0.0;
            for (name, path, last) in &mut self.files {
                let Ok(now) = std::fs::metadata(&*path).and_then(|m| m.modified()) else {
                    continue;
                };
                if *last == Some(now) {
                    continue;
                }
                *last = Some(now);
                match std::fs::read_to_string(&*path) {
                    Ok(source) => match play.play.reload(name, &source) {
                        Ok(()) => log::info!("reloaded {name}"),
                        Err(error) => log::error!("{error}"),
                    },
                    Err(error) => log::error!("{}: {error}", path.display()),
                }
            }
        }
    }

    /// The light for the next frame, `seconds` after the last: the clock's, moving on, or the
    /// fixed environment when there is no clock.
    fn light(clock: &mut Option<DayClock>, fixed: Environment, seconds: f32) -> Environment {
        match clock {
            Some(clock) => clock.tick(seconds),
            None => fixed,
        }
    }

    fn parse_args() -> Result<Options, String> {
        let mut options = Options {
            stream: false,
            budget: StreamingBudget::default(),
            scene: PathBuf::from("assets/temple.solstice"),
            capture: None,
            frames: 60,
            size: (1600, 900),
            camera: None,
            environment: default_environment(),
            settings: RendererSettings::default(),
            power: wgpu::PowerPreference::HighPerformance,
            unpack_textures: false,
            limits: None,
            capture_motion: 0.0,
            flyover: None,
            flyover_out: PathBuf::from("screenshots/flyover"),
            stability: false,
            weather: Weather::default(),
            weathered: 0.0,
            strike_at: None,
            ocean: None,
            sea_level: None,
            ponds: Vec::new(),
            clock: None,
            effects: Vec::new(),
            bob: None,
            play: false,
            game: None,
            fly: false,
            instances_near: None,
            scripts: PathBuf::from("scenes"),
            presses: Vec::new(),
            holds: Vec::new(),
            mouse_moves: Vec::new(),
        };
        // The time of day, built up from its options and made at the end.
        let (mut time, mut day_seconds, mut latitude) = (None, 0.0, None);
        let mut day = TimeOfDay::default().day;
        let floats = |text: &str| -> Result<Vec<f32>, String> {
            text.split(',')
                .map(|part| part.parse::<f32>().map_err(|e| format!("{part}: {e}")))
                .collect()
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--capture" => options.capture = Some(PathBuf::from(value()?)),
                "--frames" => {
                    options.frames = value()?.parse().map_err(|e| format!("--frames: {e}"))?
                }
                "--size" => {
                    let text = value()?;
                    let (w, h) = text.split_once('x').ok_or("--size expects WxH")?;
                    options.size = (
                        w.parse().map_err(|e| format!("--size: {e}"))?,
                        h.parse().map_err(|e| format!("--size: {e}"))?,
                    );
                }
                "--camera" => {
                    let v = floats(&value()?)?;
                    let [x, y, z, yaw, pitch] = v[..] else {
                        return Err("--camera expects x,y,z,yaw,pitch".into());
                    };
                    options.camera = Some(Camera {
                        position: Vec3::new(x, y, z),
                        yaw,
                        pitch,
                        ..Default::default()
                    });
                }
                "--sun" => {
                    let v = floats(&value()?)?;
                    let [x, y, z] = v[..] else {
                        return Err("--sun expects x,y,z".into());
                    };
                    options.environment.sun_direction = Vec3::new(x, y, z).normalize();
                }
                "--debug" => {
                    let name = value()?;
                    options.settings.debug_view = DebugView::from_name(&name)
                        .ok_or_else(|| format!("unknown debug view: {name}"))?;
                }
                "--stability" => options.stability = true,
                "--no-taa" => options.settings.temporal_accumulation = false,
                "--no-clouds" => options.settings.clouds = false,
                "--time" => {
                    let text = value()?;
                    time =
                        Some(parse_time(&text).ok_or(format!("--time expects HH:MM, not {text}"))?);
                }
                "--day" => day = value()?.parse().map_err(|e| format!("--day: {e}"))?,
                "--latitude" => {
                    latitude = Some(value()?.parse().map_err(|e| format!("--latitude: {e}"))?);
                }
                "--day-length" => {
                    day_seconds = value()?.parse().map_err(|e| format!("--day-length: {e}"))?;
                }
                "--ocean" => {
                    let name = value()?;
                    options.ocean = Some(match Ocean::by_name(&name) {
                        Some(ocean) => ocean,
                        None => {
                            let text = std::fs::read_to_string(&name).map_err(|e| {
                                let names: Vec<_> = Ocean::names().collect();
                                format!(
                                    "--ocean: {name} is neither {} nor a file: {e}",
                                    names.join(", ")
                                )
                            })?;
                            Ocean::parse(&text).map_err(|e| format!("{name}: {e}"))?
                        }
                    });
                }
                "--effect" => {
                    // NAME or FILE, then optionally @x,y,z for where it stands.
                    let text = value()?;
                    let (name, at) = match text.split_once('@') {
                        Some((name, at)) => {
                            let at = floats(at)?;
                            let [x, y, z] = at[..] else {
                                return Err("--effect expects NAME@x,y,z".into());
                            };
                            (name.to_owned(), Vec3::new(x, y, z))
                        }
                        None => (text, Vec3::ZERO),
                    };
                    let effect = match Effect::by_name(&name) {
                        Some(effect) => effect,
                        None => {
                            let text = std::fs::read_to_string(&name).map_err(|e| {
                                let names: Vec<_> = Effect::names().collect();
                                format!(
                                    "--effect: {name} is neither {} nor a file: {e}",
                                    names.join(", ")
                                )
                            })?;
                            Effect::parse(&text).map_err(|e| format!("{name}: {e}"))?
                        }
                    };
                    options.effects.push((effect, at));
                }
                "--sea-level" => {
                    options.sea_level =
                        Some(value()?.parse().map_err(|e| format!("--sea-level: {e}"))?);
                }
                "--pond" => {
                    let ponds = Pond::parse_list(&value()?).map_err(|e| format!("--pond: {e}"))?;
                    options.ponds.extend(ponds);
                }
                "--strike-at" => {
                    options.strike_at =
                        Some(value()?.parse().map_err(|e| format!("--strike-at: {e}"))?);
                }
                "--weathered" => {
                    options.weathered =
                        value()?.parse().map_err(|e| format!("--weathered: {e}"))?;
                }
                "--weather" => {
                    let name = value()?;
                    options.weather = match Weather::by_name(&name) {
                        Some(weather) => weather,
                        None => {
                            let text = std::fs::read_to_string(&name).map_err(|e| {
                                let names: Vec<_> = Weather::names().collect();
                                format!(
                                    "--weather: {name} is neither {} nor a file: {e}",
                                    names.join(", ")
                                )
                            })?;
                            Weather::parse(&text).map_err(|e| format!("{name}: {e}"))?
                        }
                    };
                }
                "--bob" => {
                    let v = floats(&value()?)?;
                    options.bob = Some(match v[..] {
                        [x, z, radius] => [x, z, radius, 0.0, 1.0],
                        [x, z, radius, height] => [x, z, radius, height, 1.0],
                        [x, z, radius, height, amplitude] => [x, z, radius, height, amplitude],
                        _ => return Err("--bob expects x,z,radius[,height[,amplitude]]".into()),
                    });
                }
                "--stream" => options.stream = true,
                "--play" => options.play = true,
                "--fly" => options.fly = true,
                "--game" => {
                    // Its scripts are watched where they were read from.
                    let folder = PathBuf::from(value()?);
                    options.play = true;
                    options.scripts.clone_from(&folder);
                    options.game = Some(folder);
                }
                "--instances-near" => {
                    options.instances_near = Some(
                        <[f32; 4]>::try_from(floats(&value()?)?)
                            .map_err(|_| "--instances-near expects x,y,z,radius")?,
                    );
                }
                "--scripts" => options.scripts = PathBuf::from(value()?),
                "--press" => {
                    let text = value()?;
                    let (action, frame) = text
                        .split_once('@')
                        .and_then(|(a, f)| Some((a.to_owned(), f.parse().ok()?)))
                        .ok_or("--press expects ACTION@FRAME")?;
                    options.presses.push((action, frame));
                }
                "--hold" => {
                    let text = value()?;
                    let parsed = text.split_once('@').and_then(|(key, range)| {
                        let (from, to) = range.split_once('-')?;
                        Some((key.to_owned(), from.parse().ok()?, to.parse().ok()?))
                    });
                    options
                        .holds
                        .push(parsed.ok_or("--hold expects KEY@FROM-TO")?);
                }
                "--mouse" => {
                    let text = value()?;
                    let parsed = text
                        .split_once('@')
                        .and_then(|(x, frame)| Some((x.parse().ok()?, frame.parse().ok()?)));
                    options
                        .mouse_moves
                        .push(parsed.ok_or("--mouse expects PIXELS@FRAME")?);
                }
                "--geometry-budget-mb" => {
                    let megabytes: u64 = value()?
                        .parse()
                        .map_err(|e| format!("--geometry-budget-mb: {e}"))?;
                    options.budget.geometry_bytes = megabytes << 20;
                }
                "--texture-budget-mb" => {
                    let megabytes: u64 = value()?
                        .parse()
                        .map_err(|e| format!("--texture-budget-mb: {e}"))?;
                    options.budget.texture_bytes = megabytes << 20;
                }
                "--flyover" => options.flyover = Some(PathBuf::from(value()?)),
                "--out" => options.flyover_out = PathBuf::from(value()?),
                "--prefetch" => {
                    options.settings.prefetch_radius =
                        value()?.parse().map_err(|e| format!("--prefetch: {e}"))?;
                }
                "--specks" => {
                    options.settings.speck_texels =
                        value()?.parse().map_err(|e| format!("--specks: {e}"))?;
                }
                "--move" => {
                    options.capture_motion =
                        value()?.parse().map_err(|e| format!("--move: {e}"))?;
                }
                "--quality" => {
                    let name = value()?;
                    let quality = Quality::from_name(&name)
                        .ok_or_else(|| format!("unknown quality: {name}"))?;
                    options.settings.apply_quality(quality);
                }
                "--low-power" => options.power = wgpu::PowerPreference::LowPower,
                "--unpack-textures" => options.unpack_textures = true,
                "--limits" => match value()?.as_str() {
                    "webgpu" => options.limits = Some(wgpu::Limits::defaults()),
                    other => return Err(format!("unknown limits: {other}")),
                },
                "--scale" => {
                    options.settings.render_scale =
                        value()?.parse().map_err(|e| format!("--scale: {e}"))?;
                }
                "--lod" => {
                    options.settings.lod_threshold =
                        value()?.parse().map_err(|e| format!("--lod: {e}"))?;
                }
                other if !other.starts_with('-') => options.scene = PathBuf::from(other),
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        if let Some(hours) = time {
            let mut clock = DayClock::new(TimeOfDay { day, hours }, day_seconds);
            if let Some(latitude) = latitude {
                clock.place.latitude = latitude;
            }
            clock.daylight = options.environment;
            options.clock = Some(clock);
        }
        Ok(options)
    }

    fn load_scene(path: &PathBuf) -> Result<LoadedScene, Box<dyn std::error::Error>> {
        let start = Instant::now();
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let scene = solstice_asset::file::decode(&bytes)?;
        log::info!(
            "loaded {} ({:.0} MB) in {:.2}s",
            path.display(),
            bytes.len() as f64 / 1.0e6,
            start.elapsed().as_secs_f32()
        );
        Ok(scene)
    }

    /// Reads a scene: all of it, or what says where the rest of it is. One whose game has
    /// actors that are in the file it shares with other scenes is read with that file, which
    /// is beside it, and the two are one scene (`solstice_viewer::shared`).
    fn read_source(options: &Options) -> Result<Source, Box<dyn std::error::Error>> {
        let path = &options.scene;
        let beside = path.with_file_name(solstice_viewer::shared::FILE);
        let missing = |error: Box<dyn std::error::Error>| {
            format!(
                "{}, which this scene is read with, could not be read: {error}",
                beside.display()
            )
        };
        Ok(if options.stream {
            let mut scene = read_manifest(path)?;
            if solstice_viewer::shared::wanted(&scene.gameplay) {
                let shared = read_manifest(&beside).map_err(missing)?;
                let files = shared.gameplay.clone();
                // The shared file's bytes are numbered on from the end of the scene's.
                let after = std::fs::metadata(path)?.len();
                let joined = solstice_asset::join::join(&mut scene, shared, after)?;
                solstice_viewer::shared::adopt(&mut scene, &files, joined)?;
            }
            Source::Streamed(scene)
        } else {
            let mut scene = load_scene(path)?;
            if solstice_viewer::shared::wanted(&scene.scene.gameplay) {
                let shared = load_scene(&beside).map_err(missing)?;
                let files = shared.scene.gameplay.clone();
                let joined = scene.join(shared)?;
                solstice_viewer::shared::adopt(&mut scene.scene, &files, joined)?;
            }
            Source::Loaded(scene)
        })
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn"),
        )
        .init();
        let options = parse_args()?;
        let scene = read_source(&options)?;
        if let Some(near) = options.instances_near {
            scene.list_instances(near);
            return Ok(());
        }
        if options.flyover.is_some() {
            let Source::Loaded(scene) = &scene else {
                return Err("flyovers load the whole scene, so they can't use --stream".into());
            };
            flyover(&options, scene)
        } else if options.capture.is_some() {
            capture(&options, &scene)
        } else {
            let event_loop = EventLoop::new()?;
            let mut app = App {
                options,
                scene,
                state: None,
                input: CameraInput::default(),
                keys: HashSet::new(),
                captured_mouse: false,
                weather_index: 0,
            };
            event_loop.run_app(&mut app)?;
            Ok(())
        }
    }

    fn create_instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            // WGPU_BACKEND picks another, such as dx12, which is what Chrome draws with on Windows.
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
    }

    /// Renders offscreen, saves a PNG, and fails loudly on any GPU validation error.
    fn capture(options: &Options, scene: &Source) -> Result<(), Box<dyn std::error::Error>> {
        let path = options
            .capture
            .as_ref()
            .expect("capture mode requires a path");
        let mut gpu = pollster::block_on(Gpu::request_within(
            &create_instance(),
            None,
            options.power,
            options.limits.as_ref(),
        ))?;
        gpu.block_compressed_textures &= !options.unpack_textures;

        let errors: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().expect("error sink").push(error.to_string());
            }));

        let start = Instant::now();
        let (mut renderer, mut reader) = scene.renderer(&gpu, CAPTURE_FORMAT, options);
        log::info!("renderer created in {:.2}s", start.elapsed().as_secs_f32());
        let mut bob = bobbing(options, &mut renderer);
        let mut play = start_play(options, scene, &mut renderer);

        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d {
                width: options.size.0,
                height: options.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let start = Instant::now();
        let mut slowest = 0.0_f32;
        // What each section of the frame took, in all and at most: a redraw of a cached view
        // costs one frame in many, which the last frame's times alone seldom show.
        let mut slowest_sections = [0.0_f32; solstice_render::profiler::SECTIONS.len()];
        let mut summed_sections = [0.0_f32; solstice_render::profiler::SECTIONS.len()];
        let mut timed_frames = 0u32;
        let mut last_time = Instant::now();
        let mut camera = options
            .camera
            .unwrap_or_else(|| initial_camera(scene.viewpoint()));
        if let Some(reader) = &mut reader {
            settle(&gpu, &mut renderer, reader, &camera, &view, options)?;
        }
        let mut clock = options.clock;
        for frame in 0..options.frames {
            if options.strike_at == Some(frame) {
                renderer.strike_lightning();
            }
            camera.position += camera.right() * options.capture_motion;
            if let Some(reader) = &mut reader {
                reader.pump(&mut renderer);
            }
            let environment = light(&mut clock, options.environment, 1.0 / 60.0);
            if let Some(bob) = &mut bob {
                bob.advance(&mut renderer, 1.0 / 60.0);
            }
            if let Some(play) = &mut play {
                for (action, at) in &options.presses {
                    if *at == frame {
                        play.play.press(action);
                    }
                }
                for (key, from, to) in &options.holds {
                    if frame == *from {
                        play.devices.key(key, true);
                    } else if frame == *to {
                        play.devices.key(key, false);
                    }
                }
                for (pixels, at) in &options.mouse_moves {
                    if *at == frame {
                        play.devices.mouse_motion(*pixels, 0.0);
                    }
                }
                log_messages(play.frame(&mut renderer, &mut camera, 1.0 / 60.0));
            }
            renderer.render(&gpu, &camera, &environment, &view, options.size, 1.0 / 60.0);
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            if frame > 10 {
                slowest = slowest.max(last_time.elapsed().as_secs_f32());
                if let Some(times) = renderer.section_times() {
                    timed_frames += 1;
                    for (index, ms) in times.into_iter().enumerate() {
                        slowest_sections[index] = slowest_sections[index].max(ms);
                        summed_sections[index] += ms;
                    }
                }
            }
            last_time = Instant::now();
        }
        if play.is_some() {
            let p = camera.position;
            log::info!("the camera ended at {:.2}, {:.2}, {:.2}", p.x, p.y, p.z);
        }
        let average = start.elapsed().as_secs_f32() / options.frames.max(1) as f32;

        save_png(&gpu, &target, options.size, path)?;

        let stats = renderer.stats();
        if reader.is_some() {
            let streaming = stats.streaming;
            println!(
                "pages: {} of {} installed in a pool of {} ({} MiB), {} evicted, {} short of room",
                streaming.installed,
                streaming.pages,
                streaming.slots,
                streaming.pool_bytes >> 20,
                streaming.evictions,
                streaming.starved,
            );
            if let Some(bands) = streaming.bands {
                let [near, middle, far] = [bands[0], bands[1], bands[2]];
                let spare = bands[3..]
                    .iter()
                    .fold((0, 0), |sum, band| (sum.0 + band.0, sum.1 + band.1));
                println!(
                    "bands: near {}/{}, middle {}/{}, far {}/{}, spare {}/{}",
                    near.0, near.1, middle.0, middle.1, far.0, far.1, spare.0, spare.1
                );
            }
            let memory = renderer.scene_bytes();
            println!(
                "scene memory: {:.1} MiB of records, {:.1} MiB of pages, {:.1} MiB of textures | textures: {}, {} at middle, {} at top",
                memory.records as f64 / 1048576.0,
                memory.pages as f64 / 1048576.0,
                memory.textures as f64 / 1048576.0,
                renderer.texture_summary(),
                streaming.middle_layers,
                streaming.top_layers,
            );
        }
        println!(
            "avg {:.2} ms, worst {:.2} ms | {} clusters ({} software), {} triangles of {} source ({} unique), {} instances",
            average * 1000.0,
            slowest * 1000.0,
            stats.clusters,
            stats.software_clusters,
            stats.triangles,
            stats.source_triangles,
            stats.unique_triangles,
            stats.instances,
        );
        if stats.particles.emitters > 0 {
            let particles = stats.particles;
            println!(
                "particles: {} alive in {} emitters, {} of {} slots reserved, {} spawned, {} dropped",
                particles.alive,
                particles.emitters,
                particles.reserved,
                particles.capacity,
                particles.spawned,
                particles.dropped,
            );
        }
        println!(
            "last shadow cascade: {} clusters, {} triangles",
            stats.shadow_clusters, stats.shadow_triangles
        );
        if stats.dropped > 0 || stats.shadow_dropped > 0 {
            println!(
                "WARNING: the cull lists overflowed: {} dropped in the main view, {} in the last shadow cascade redrawn",
                stats.dropped, stats.shadow_dropped
            );
        }

        if let Some(times) = renderer.section_times() {
            let parts: Vec<String> = solstice_render::profiler::SECTIONS
                .iter()
                .zip(times)
                .map(|(name, ms)| format!("{name} {ms:.2}"))
                .collect();
            println!(
                "gpu ms: {} | total {:.2}",
                parts.join(", "),
                times.iter().sum::<f32>()
            );
            let listed = |times: [f32; solstice_render::profiler::SECTIONS.len()]| {
                let parts: Vec<String> = solstice_render::profiler::SECTIONS
                    .iter()
                    .zip(times)
                    .filter(|(_, ms)| *ms >= 0.005)
                    .map(|(name, ms)| format!("{name} {ms:.2}"))
                    .collect();
                format!(
                    "{} | total {:.2}",
                    parts.join(", "),
                    times.iter().sum::<f32>()
                )
            };
            let frames = timed_frames.max(1) as f32;
            println!(
                "gpu ms on average: {}",
                listed(summed_sections.map(|ms| ms / frames))
            );
            println!("gpu ms at most: {}", listed(slowest_sections));
        }

        let errors = errors.lock().expect("error sink");
        if errors.is_empty() {
            println!("saved {} with no GPU errors", path.display());
            Ok(())
        } else {
            Err(format!("{} GPU errors, first:\n{}", errors.len(), errors[0]).into())
        }
    }

    /// Renders from one spot until streaming has nothing left to do, so a capture shows what
    /// the view settles on rather than how far loading got.
    fn settle(
        gpu: &Gpu,
        renderer: &mut Renderer,
        reader: &mut PageReader,
        camera: &Camera,
        view: &wgpu::TextureView,
        options: &Options,
    ) -> Result<(), Box<dyn std::error::Error>> {
        const QUIET_FRAMES: u32 = 30;
        const MAX_FRAMES: u32 = 20000;
        let start = Instant::now();
        let mut quiet = 0;
        let mut frames = 0;
        while quiet < QUIET_FRAMES && frames < MAX_FRAMES {
            let before = renderer.stats().streaming.installed;
            reader.pump(renderer);
            renderer.render(
                gpu,
                camera,
                &options
                    .clock
                    .map_or(options.environment, |clock| clock.environment()),
                view,
                options.size,
                1.0 / 60.0,
            );
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            let stats = renderer.stats().streaming;
            let textures_busy = stats.texture_fetches > 0;
            // Pages may be left waiting with nowhere to go, so those don't count as busy.
            if stats.installed == before && !textures_busy && reader.is_idle() {
                quiet += 1;
            } else {
                quiet = 0;
            }
            frames += 1;
        }
        let stats = renderer.stats().streaming;
        println!(
            "streaming settled after {frames} frames, {:.2}s: {} of {} pages",
            start.elapsed().as_secs_f32(),
            stats.installed,
            stats.pages,
        );
        Ok(())
    }

    /// Flies a camera path at a fixed 60 steps per second. Writes an image for each second, plus
    /// `frames.csv` with the time of every frame. Each frame waits for the GPU to finish, so its
    /// time is real GPU work, not hidden by queuing, and image saving is left out of the timing.
    fn flyover(options: &Options, scene: &LoadedScene) -> Result<(), Box<dyn std::error::Error>> {
        const STEP: f32 = 1.0 / 60.0;
        use solstice_render::profiler::SECTIONS;

        let text = std::fs::read_to_string(
            options
                .flyover
                .as_ref()
                .expect("flyover mode requires a path"),
        )?;
        let path = solstice_viewer::camera_path::CameraPath::parse(&text)?;
        std::fs::create_dir_all(&options.flyover_out)?;

        let mut gpu = pollster::block_on(Gpu::request_within(
            &create_instance(),
            None,
            options.power,
            options.limits.as_ref(),
        ))?;
        gpu.block_compressed_textures &= !options.unpack_textures;
        let errors: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let sink = errors.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                sink.lock().expect("error sink").push(error.to_string());
            }));
        log::info!("flying on {}", gpu.adapter_info.name);

        let mut renderer = Renderer::new(&gpu, scene, CAPTURE_FORMAT, options.settings);
        renderer.set_weather(options.weather, 0.0);
        renderer.advance_weather(options.weathered);
        if options.ocean.is_some() {
            renderer.set_ocean(options.ocean, 0.0);
        }
        if let Some(level) = options.sea_level {
            renderer.set_sea_level(level);
        }
        if !options.ponds.is_empty() {
            renderer.set_ponds(&options.ponds);
        }
        spawn_effects(&mut renderer, options);
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flyover"),
            size: wgpu::Extent3d {
                width: options.size.0,
                height: options.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let frames = (path.duration() / STEP) as u32;
        let mut records: Vec<(f32, f32, u32, [f32; SECTIONS.len()])> =
            Vec::with_capacity(frames as usize);
        let mut previous_grid: Option<Vec<f32>> = None;
        let mut changes: Vec<(f32, GridChange)> = Vec::new();
        let mut clock = options.clock;
        let (mut dropped, mut shadow_dropped) = (0, 0);
        for frame in 0..=frames {
            let time = frame as f32 * STEP;
            let start = Instant::now();
            let environment = light(&mut clock, options.environment, STEP);
            renderer.render(
                &gpu,
                &path.sample(time),
                &environment,
                &view,
                options.size,
                STEP,
            );
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            let elapsed = start.elapsed().as_secs_f32() * 1000.0;
            let sections = renderer.section_times().unwrap_or_default();
            let stats = renderer.stats();
            records.push((time, elapsed, stats.triangles, sections));
            dropped = dropped.max(stats.dropped);
            shadow_dropped = shadow_dropped.max(stats.shadow_dropped);
            if options.stability {
                let pixels = read_pixels(&gpu, &target, options.size)?;
                let grid = luma_grid(&pixels, options.size);
                if let Some(previous) = &previous_grid {
                    changes.push((time, grid_change(previous, &grid)));
                }
                previous_grid = Some(grid);
            }
            if frame % 60 == 0 {
                let second = frame / 60;
                save_png(
                    &gpu,
                    &target,
                    options.size,
                    &options.flyover_out.join(format!("{second:03}.png")),
                )?;
            }
        }

        let mut csv = format!("time,ms,triangles,{}\n", SECTIONS.join(","));
        for (time, ms, triangles, sections) in &records {
            let parts: Vec<String> = sections.iter().map(|s| format!("{s:.3}")).collect();
            csv += &format!("{time:.3},{ms:.3},{triangles},{}\n", parts.join(","));
        }
        std::fs::write(options.flyover_out.join("frames.csv"), csv)?;

        // The first frames build everything from nothing, which is loading, not stutter.
        let flight = &records[records.len().min(30)..];
        let mut sorted: Vec<f32> = flight.iter().map(|r| r.1).collect();
        sorted.sort_by(f32::total_cmp);
        let percentile = |p: f32| sorted[((sorted.len() - 1) as f32 * p) as usize];
        let count_over = |limit: f32| flight.iter().filter(|r| r.1 > limit).count();
        println!(
            "{} frames | avg {:.2} ms | p50 {:.2} p95 {:.2} p99 {:.2} max {:.2} | over 16.7 ms: {}, over 33 ms: {} | startup {:.0} ms",
            flight.len(),
            sorted.iter().sum::<f32>() / sorted.len() as f32,
            percentile(0.5),
            percentile(0.95),
            percentile(0.99),
            sorted[sorted.len() - 1],
            count_over(16.7),
            count_over(33.3),
            records[0].1,
        );
        for (index, name) in SECTIONS.iter().enumerate() {
            let (sum, max) = flight.iter().fold((0.0, 0.0_f32), |(sum, max), r| {
                (sum + r.3[index], max.max(r.3[index]))
            });
            println!(
                "  {name:10} avg {:7.2} ms   max {:7.2} ms",
                sum / flight.len() as f32,
                max
            );
        }
        let mut worst: Vec<_> = flight.iter().collect();
        worst.sort_by(|a, b| b.1.total_cmp(&a.1));
        for record in worst.iter().take(4) {
            println!(
                "  slow frame at {:5.2}s: {:6.2} ms, {} triangles",
                record.0, record.1, record.2
            );
        }
        if options.stability {
            report_stability(&changes);
        }

        if dropped > 0 || shadow_dropped > 0 {
            println!(
                "WARNING: the cull lists overflowed: up to {dropped} dropped in the main view, {shadow_dropped} in the last shadow cascade redrawn each frame"
            );
        }
        let errors = errors.lock().expect("error sink");
        if errors.is_empty() {
            println!("no GPU errors");
            Ok(())
        } else {
            Err(format!("{} GPU errors, first:\n{}", errors.len(), errors[0]).into())
        }
    }

    /// How much a frame differs from the one before, on a coarse grid of average brightness.
    #[derive(Clone, Copy)]
    struct GridChange {
        mean: f32,
        worst_tile: f32,
        worst_at: (usize, usize),
    }

    const GRID: (usize, usize) = (48, 27);

    fn luma_grid(pixels: &[u8], size: (u32, u32)) -> Vec<f32> {
        let (width, height) = (size.0 as usize, size.1 as usize);
        let mut sums = vec![0.0_f32; GRID.0 * GRID.1];
        let mut counts = vec![0u32; GRID.0 * GRID.1];
        for y in 0..height {
            for x in 0..width {
                let p = &pixels[(y * width + x) * 4..][..3];
                let tile = (y * GRID.1 / height) * GRID.0 + x * GRID.0 / width;
                sums[tile] += 0.3 * f32::from(p[0]) + 0.6 * f32::from(p[1]) + 0.1 * f32::from(p[2]);
                counts[tile] += 1;
            }
        }
        sums.iter()
            .zip(&counts)
            .map(|(s, &c)| s / c.max(1) as f32)
            .collect()
    }

    fn grid_change(a: &[f32], b: &[f32]) -> GridChange {
        let mut sum = 0.0;
        let mut worst = 0.0_f32;
        let mut worst_at = (0, 0);
        for (index, (x, y)) in a.iter().zip(b).enumerate() {
            let d = (x - y).abs();
            sum += d;
            if d > worst {
                worst = d;
                worst_at = (index % GRID.0, index / GRID.0);
            }
        }
        GridChange {
            mean: sum / a.len() as f32,
            worst_tile: worst,
            worst_at,
        }
    }

    /// Camera motion changes every frame steadily. A flicker or pop is a jump well above the
    /// frames around it, so each frame is compared with the median of its neighbours.
    fn report_stability(changes: &[(f32, GridChange)]) {
        const WINDOW: usize = 15;
        const POP_RATIO: f32 = 3.0;
        const POP_MINIMUM: f32 = 3.0;
        let flight = &changes[changes.len().min(30)..];
        let mut pops: Vec<(f32, f32, f32, (usize, usize))> = Vec::new();
        for index in 0..flight.len() {
            let start = index.saturating_sub(WINDOW);
            let end = (index + WINDOW + 1).min(flight.len());
            let mut around: Vec<f32> = flight[start..end].iter().map(|c| c.1.worst_tile).collect();
            around.sort_by(f32::total_cmp);
            let median = around[around.len() / 2];
            let here = flight[index].1.worst_tile;
            if here > median * POP_RATIO && here > POP_MINIMUM {
                pops.push((flight[index].0, here, median, flight[index].1.worst_at));
            }
        }
        let mean = flight.iter().map(|c| c.1.mean).sum::<f32>() / flight.len().max(1) as f32;
        let worst = flight.iter().map(|c| c.1.worst_tile).fold(0.0, f32::max);
        let worst_time = flight
            .iter()
            .find(|c| c.1.worst_tile == worst)
            .map_or(0.0, |c| c.0);
        println!(
            "stability: mean change {mean:.2}/255 per frame, worst tile {worst:.1}/255 at {worst_time:.2}s, {} sudden jumps",
            pops.len()
        );
        pops.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (time, here, median, at) in pops.iter().take(12) {
            println!(
                "  jump at {time:5.2}s: tile {at:?} of {GRID:?} changed {here:.1}, typical there {median:.1}"
            );
        }
    }

    fn read_pixels(
        gpu: &Gpu,
        texture: &wgpu::Texture,
        size: (u32, u32),
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let row_bytes = size.0 * 4;
        let aligned = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture staging"),
            size: u64::from(aligned) * u64::from(size.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned),
                    rows_per_image: Some(size.1),
                },
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        staging.slice(..).map_async(wgpu::MapMode::Read, |result| {
            result.expect("mapping the capture buffer failed");
        });
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;

        let data = staging.slice(..).get_mapped_range()?;
        let mut pixels = Vec::with_capacity((row_bytes * size.1) as usize);
        for row in data.chunks_exact(aligned as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(data);
        Ok(pixels)
    }

    fn save_png(
        gpu: &Gpu,
        texture: &wgpu::Texture,
        size: (u32, u32),
        path: &PathBuf,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let pixels = read_pixels(gpu, texture, size)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        Ok(())
    }

    struct WindowState {
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        gpu: Gpu,
        renderer: Renderer,
        reader: Option<PageReader>,
        camera: FlyCamera,
        bob: Option<Bobbing>,
        play: Option<(PlayMode, ScriptWatch)>,
        last_frame: Instant,
        title_timer: f32,
        frames_since_title: u32,
        /// Frames a second, as last measured, and how long since the dashboard was written.
        fps: f32,
        hud_timer: f32,
    }

    struct App {
        options: Options,
        scene: Source,
        state: Option<WindowState>,
        input: CameraInput,
        keys: HashSet<KeyCode>,
        captured_mouse: bool,
        /// Which built-in weather the C key moved to last.
        weather_index: usize,
    }

    impl App {
        fn create_state(
            &self,
            event_loop: &ActiveEventLoop,
        ) -> Result<WindowState, Box<dyn std::error::Error>> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("Solstice")
                        .with_inner_size(PhysicalSize::new(
                            self.options.size.0,
                            self.options.size.1,
                        )),
                )?,
            );
            let instance = create_instance();
            let surface = instance.create_surface(window.clone())?;
            let gpu = pollster::block_on(Gpu::request_within(
                &instance,
                Some(&surface),
                self.options.power,
                self.options.limits.as_ref(),
            ))?;

            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
                .ok_or("this surface cannot be presented to")?;
            config.format = config.format.remove_srgb_suffix();
            config.present_mode = wgpu::PresentMode::AutoVsync;
            surface.configure(&gpu.device, &config);

            let (mut renderer, reader) = self.scene.renderer(&gpu, config.format, &self.options);
            let bob = bobbing(&self.options, &mut renderer);
            let play = start_play(&self.options, &self.scene, &mut renderer).map(|play| {
                let watch = ScriptWatch::new(&play, &self.options.scripts);
                (play, watch)
            });
            Ok(WindowState {
                bob,
                play,
                reader,
                window,
                surface,
                config,
                gpu,
                renderer,
                camera: FlyCamera::new(
                    self.options
                        .camera
                        .unwrap_or_else(|| initial_camera(self.scene.viewpoint())),
                ),
                last_frame: Instant::now(),
                title_timer: 0.0,
                frames_since_title: 0,
                fps: 0.0,
                hud_timer: 0.0,
            })
        }

        fn axis(&self, positive: KeyCode, negative: KeyCode) -> f32 {
            f32::from(u8::from(self.keys.contains(&positive)))
                - f32::from(u8::from(self.keys.contains(&negative)))
        }

        fn draw(&mut self) {
            let forward = self.axis(KeyCode::KeyW, KeyCode::KeyS);
            let right = self.axis(KeyCode::KeyD, KeyCode::KeyA);
            let up = self.axis(KeyCode::KeyE, KeyCode::KeyQ);
            let fast = self.keys.contains(&KeyCode::ShiftLeft);
            let look = std::mem::take(&mut self.input);
            let (fixed, clock) = (self.options.environment, &mut self.options.clock);
            let Some(state) = self.state.as_mut() else {
                return;
            };

            if let Some(reader) = &mut state.reader {
                reader.pump(&mut state.renderer);
            }

            let now = Instant::now();
            let delta = (now - state.last_frame).as_secs_f32();
            state.last_frame = now;
            let environment = light(clock, fixed, delta);
            if let Some(bob) = &mut state.bob {
                bob.advance(&mut state.renderer, delta);
            }
            // Playing as someone, the keys and the mouse are theirs; otherwise, or having left
            // them with V, they fly the camera.
            let playing_as_someone = state
                .play
                .as_ref()
                .is_some_and(|(p, _)| p.play.has_camera());
            if !playing_as_someone {
                state.camera.update(
                    &CameraInput {
                        forward,
                        right,
                        up,
                        fast,
                        ..look
                    },
                    delta,
                );
            }
            let mut driving = false;
            if let Some((play, watch)) = &mut state.play {
                watch.check(play, delta);
                log_messages(play.frame(&mut state.renderer, &mut state.camera.camera, delta));
                // Driving, the title is the dashboard: speed, gear, revs and lap times.
                if let Some(hud) = play.play.hud() {
                    driving = true;
                    state.hud_timer += delta;
                    if state.hud_timer > 0.1 {
                        state.hud_timer = 0.0;
                        state.window.set_title(&format!(
                            "{}   |   {:.0} fps",
                            hud.line(),
                            state.fps
                        ));
                    }
                }
            }

            state.title_timer += delta;
            state.frames_since_title += 1;
            if state.title_timer > 0.5 {
                let stats = state.renderer.stats();
                state.fps = state.frames_since_title as f32 / state.title_timer;
                if !driving {
                    state.window.set_title(&format!(
                        "Solstice | {:.0} fps | {:.2}M tris of {:.0}M | {} clusters ({} sw)",
                        state.fps,
                        stats.triangles as f32 / 1.0e6,
                        stats.source_triangles as f32 / 1.0e6,
                        stats.clusters,
                        stats.software_clusters,
                    ));
                }
                state.title_timer = 0.0;
                state.frames_since_title = 0;
            }

            let texture = match state.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture)
                | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    state.surface.configure(&state.gpu.device, &state.config);
                    return;
                }
                _ => return,
            };
            let view = texture.texture.create_view(&Default::default());
            state.renderer.render(
                &state.gpu,
                &state.camera.camera,
                &environment,
                &view,
                (state.config.width, state.config.height),
                delta,
            );
            state.gpu.queue.present(texture);
        }
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.state.is_some() {
                return;
            }
            match self.create_state(event_loop) {
                Ok(state) => self.state = Some(state),
                Err(error) => {
                    log::error!("{error}");
                    event_loop.exit();
                }
            }
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    if let Some(state) = self.state.as_mut() {
                        if size.width > 0 && size.height > 0 {
                            state.config.width = size.width;
                            state.config.height = size.height;
                            state.surface.configure(&state.gpu.device, &state.config);
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        // winit's key names are the browser's `KeyboardEvent.code` names.
                        if let Some((play, _)) = self.state.as_mut().and_then(|s| s.play.as_mut()) {
                            if !event.repeat {
                                let down = event.state == ElementState::Pressed;
                                play.devices.key(&format!("{code:?}"), down);
                                // Leaves whoever is played to fly the camera, and comes back.
                                if down && code == KeyCode::KeyV && play.play.has_player() {
                                    let flying = !play.play.flying();
                                    play.play.set_flying(flying);
                                    log::info!("{}", if flying { "flying" } else { "playing" });
                                }
                            }
                        }
                        // While someone is played, and not left to fly the camera, the keys
                        // are the game's: C changes a car's view, so it is not the weather's.
                        let played = self
                            .state
                            .as_ref()
                            .and_then(|s| s.play.as_ref())
                            .is_some_and(|(play, _)| play.play.has_player() && !play.play.flying());
                        match event.state {
                            ElementState::Pressed => {
                                self.keys.insert(code);
                                if code == KeyCode::Escape {
                                    self.set_mouse_capture(false);
                                }
                                // Cycles through the built-in weather, easing into each.
                                if code == KeyCode::KeyC && !played {
                                    self.next_weather();
                                }
                                // Moves the clock on an hour, starting one if there is none.
                                if code == KeyCode::KeyT && !played {
                                    let fixed = self.options.environment;
                                    let clock = self.options.clock.get_or_insert_with(|| {
                                        let mut clock = DayClock::new(TimeOfDay::default(), 0.0);
                                        clock.daylight = fixed;
                                        clock
                                    });
                                    clock.time.advance(1.0);
                                    log::info!("time {:.0}:00", clock.time.hours.floor());
                                }
                            }
                            ElementState::Released => {
                                self.keys.remove(&code);
                            }
                        }
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    if let Some((play, _)) = self.state.as_mut().and_then(|s| s.play.as_mut()) {
                        let name = match button {
                            MouseButton::Left => Some("left"),
                            MouseButton::Right => Some("right"),
                            MouseButton::Middle => Some("middle"),
                            _ => None,
                        };
                        // The click that captures the mouse is not a click in the game.
                        if let (Some(name), true) = (name, self.captured_mouse) {
                            play.devices
                                .mouse_button(name, state == ElementState::Pressed);
                        }
                    }
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.set_mouse_capture(true);
                    }
                }
                WindowEvent::Focused(false) => {
                    self.keys.clear();
                    if let Some((play, _)) = self.state.as_mut().and_then(|s| s.play.as_mut()) {
                        play.devices.release_all();
                    }
                }
                WindowEvent::RedrawRequested => {
                    self.draw();
                    if let Some(state) = &self.state {
                        state.window.request_redraw();
                    }
                }
                _ => {}
            }
        }

        fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
            if let (DeviceEvent::MouseMotion { delta }, true) = (event, self.captured_mouse) {
                self.input.look_yaw -= delta.0 as f32 * 0.002;
                self.input.look_pitch -= delta.1 as f32 * 0.002;
                if let Some((play, _)) = self.state.as_mut().and_then(|s| s.play.as_mut()) {
                    play.devices.mouse_motion(delta.0 as f32, delta.1 as f32);
                }
            }
        }
    }

    impl App {
        /// Eases into the next built-in weather, over half a minute, as a front would bring it.
        fn next_weather(&mut self) {
            let Some(state) = &mut self.state else {
                return;
            };
            let names: Vec<&str> = Weather::names().collect();
            self.weather_index = (self.weather_index + 1) % names.len();
            let name = names[self.weather_index];
            log::info!("weather: {name}");
            state
                .renderer
                .set_weather(Weather::by_name(name).expect("listed weather"), 30.0);
        }

        fn set_mouse_capture(&mut self, capture: bool) {
            let Some(state) = &self.state else { return };
            let mode = if capture {
                CursorGrabMode::Locked
            } else {
                CursorGrabMode::None
            };
            if state.window.set_cursor_grab(mode).is_err() && capture {
                let _ = state.window.set_cursor_grab(CursorGrabMode::Confined);
            }
            state.window.set_cursor_visible(!capture);
            self.captured_mouse = capture;
        }
    }
}
