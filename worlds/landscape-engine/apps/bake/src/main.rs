//! Bakes scenes into Solstice scene files: the procedural demo, or an imported model.
//!
//! ```text
//! bake [--quality draft|web|high] [--output FILE]
//! bake --gltf MODEL.glb [--fit METRES] [--ground] [--texture-size N] [--output FILE]
//! bake --web-split SCENE.solstice --output SCENE.web
//! bake --large-gltf SCENE.gltf [--region X,Y,Z,RADIUS] [--no-textures] [--glow STRENGTH] [--texture-size N] [--output FILE]
//! bake --scene SCENE.txt [--texture-size N] [--output FILE]
//! bake --gallery [--texture-size N] [--output FILE]
//! bake --forest [--texture-size N] [--output FILE]
//! bake --plants [--only] [--species FILE.species]... [--grass FILE.grass]... [--texture-size N] [--output FILE]
//! bake --check-character CHARACTER.glb [--roles RIG.roles]
//! bake --scene SCENE.txt --show-collision [--output FILE]
//! bake --project SCENE.scene [--texture-size N] [--output FILE]
//! bake --convert SCENE.txt
//! bake --surfaces FOLDER [--texture-size N]
//! ```
//!
//! `--project` bakes a scene of an editor project (`docs/editor.md`), to the project's
//! `build/SCENE.solstice` unless `--output` says otherwise. `--convert` bakes nothing: it writes
//! a scene's text as a project's `SCENE.scene` beside it, and lists it in the `solstice.project`
//! there.
//!
//! `--surfaces` writes the surfaces a land is laid with, each as a scene file holding one
//! compressed texture layer, for the editor's bake in the browser to take ready-made rather than
//! spend minutes making (`solstice_bake::textures`): `surface-NAME-0-SIZE.solstice`.
//!
//! `--check-character` bakes nothing: it checks a rigged character as the export tool wrote it
//! (`tools/blender/solstice_export.py`), prints what it holds and what is wrong with it, and
//! fails if anything is.
//!
//! `--plants` grows every built-in species and breed of grass, then each species and grass file
//! given, so either can be edited and looked at without touching code.
//!
//! Textures are block compressed unless `--raw-textures` is given.

mod collision_view;
mod forest;
mod gallery;
mod kit;
mod plants;
mod scene;

use scene::Quality;
use solstice_asset::SceneAsset;
use solstice_bake::{project, scene_file};
use solstice_import::{import_gltf, ImportOptions};
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Options {
    quality: Quality,
    output: PathBuf,
    gltf: Option<PathBuf>,
    scene_file: Option<PathBuf>,
    /// A scene of an editor project.
    project_scene: Option<PathBuf>,
    /// A scene's text to convert into a project's scene.
    convert: Option<PathBuf>,
    /// Whether `--output` was given.
    output_given: bool,
    import: ImportOptions,
    /// Replaces the size a scene file asks for.
    scene_texture_size: Option<u32>,
    raw_textures: bool,
    /// Shows every procedural surface, instead of the demo scene.
    gallery: bool,
    /// The procedural meadow and wood, instead of the demo scene.
    forest: bool,
    /// A folder to write the land's surfaces to, ready-made for the editor.
    surfaces: Option<PathBuf>,
    /// Bakes what the scene collides with, instead of how it looks.
    show_collision: bool,
    /// A row of every species, instead of the demo scene.
    plants: bool,
    /// Only the species and grass files given, without the built-in ones.
    only: bool,
    /// Species files to add to the row of plants.
    species: Vec<PathBuf>,
    /// Grass files to add to the row of grass.
    grass: Vec<PathBuf>,
    /// A scene too large for memory, baked straight to the output.
    large_gltf: Option<PathBuf>,
    /// A baked scene to write the browser's file for.
    web_split: Option<PathBuf>,
    large: solstice_import::streamed::LargeOptions,
    /// A character to check, and its rig's role file.
    check_character: Option<PathBuf>,
    roles: Option<PathBuf>,
    /// A truck built of pieces to pack: its actor file (`kit.rs`).
    kit: Option<PathBuf>,
}

/// Writes the file a browser reads for a scene too large for its memory: the manifest without
/// its cluster records and pinned pages, which follow it uncompressed for the page to hand over
/// in parts. The scene's other pages stay in the scene file.
fn web_split(path: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use solstice_asset::file;
    use solstice_asset::paged::ExternalTables;
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut input = std::fs::File::open(path)?;
    let mut head = [0; file::HEADER_BYTES];
    input.read_exact(&mut head)?;
    let header = file::decode_header(&head)?;
    input.seek(SeekFrom::Start(header.manifest_offset))?;
    let mut packed = vec![0; header.manifest_bytes as usize];
    input.read_exact(&mut packed)?;
    let manifest = file::inflate(&packed)?;
    drop(packed);
    let mut scene =
        file::decode_manifest(&manifest, header.manifest_offset + header.manifest_bytes)?;
    drop(manifest);
    if scene.external.is_some() {
        return Err("this file is split already: split the .solstice file it was made from".into());
    }
    if !scene.skeletons.is_empty() {
        return Err("a scene with skinned meshes can't be split".into());
    }
    let clusters = std::mem::take(&mut scene.clusters);
    let pinned = std::mem::take(&mut scene.pinned_data);
    scene.external = Some(ExternalTables {
        clusters: clusters.len() as u64,
        pinned_bytes: pinned.len() as u64,
    });
    let start = file::encode_header_and_manifest(&scene);
    let mut out = std::io::BufWriter::with_capacity(1 << 22, std::fs::File::create(output)?);
    out.write_all(&start)?;
    // In the order pages hold them, so that a page's records are one run of the file.
    for set in &scene.sets {
        let range = set.first_cluster as usize..(set.first_cluster + set.cluster_count) as usize;
        out.write_all(file::cluster_bytes(&clusters[range]))?;
    }
    out.write_all(&pinned)?;
    out.flush()?;
    println!(
        "wrote {}: manifest {:.1} MB, {} clusters, {:.1} MB of pinned pages",
        output.display(),
        start.len() as f64 / 1e6,
        clusters.len(),
        pinned.len() as f64 / 1e6,
    );
    Ok(())
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        quality: Quality::High,
        output: PathBuf::from("assets/temple.solstice"),
        gltf: None,
        scene_file: None,
        project_scene: None,
        convert: None,
        output_given: false,
        import: ImportOptions::default(),
        scene_texture_size: None,
        raw_textures: false,
        gallery: false,
        forest: false,
        surfaces: None,
        show_collision: false,
        plants: false,
        only: false,
        species: Vec::new(),
        grass: Vec::new(),
        large_gltf: None,
        web_split: None,
        large: Default::default(),
        check_character: None,
        roles: None,
        kit: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--quality" => {
                options.quality = match value()?.as_str() {
                    "draft" => Quality::Draft,
                    "web" => Quality::Web,
                    "high" => Quality::High,
                    other => return Err(format!("unknown quality: {other}")),
                };
            }
            "--output" => {
                options.output = PathBuf::from(value()?);
                options.output_given = true;
            }
            "--scene" => options.scene_file = Some(PathBuf::from(value()?)),
            "--kit" => options.kit = Some(PathBuf::from(value()?)),
            "--project" => options.project_scene = Some(PathBuf::from(value()?)),
            "--convert" => options.convert = Some(PathBuf::from(value()?)),
            "--check-character" => options.check_character = Some(PathBuf::from(value()?)),
            "--roles" => options.roles = Some(PathBuf::from(value()?)),
            "--gltf" => options.gltf = Some(PathBuf::from(value()?)),
            "--large-gltf" => options.large_gltf = Some(PathBuf::from(value()?)),
            "--web-split" => options.web_split = Some(PathBuf::from(value()?)),
            "--no-textures" => options.large.textures = false,
            "--glow" => {
                options.large.glow_strength =
                    value()?.parse().map_err(|e| format!("--glow: {e}"))?;
            }
            "--region" => {
                let text = value()?;
                let numbers = text
                    .split(',')
                    .map(|n| n.parse::<f32>().map_err(|e| format!("--region: {e}")))
                    .collect::<Result<Vec<_>, _>>()?;
                let [x, y, z, radius] = numbers[..] else {
                    return Err("--region takes X,Y,Z,RADIUS".to_owned());
                };
                options.large.region = Some((glam::Vec3::new(x, y, z), radius));
            }
            "--fit" => {
                let metres = value()?.parse().map_err(|e| format!("--fit: {e}"))?;
                options.import.fit_size = Some(metres);
            }
            "--ground" => options.import.ground = true,
            "--texture-size" => {
                let size = value()?
                    .parse()
                    .map_err(|e| format!("--texture-size: {e}"))?;
                options.import.texture_size = size;
                options.large.texture_size = size;
                options.scene_texture_size = Some(size);
            }
            "--raw-textures" => options.raw_textures = true,
            "--gallery" => options.gallery = true,
            "--forest" => options.forest = true,
            "--surfaces" => options.surfaces = Some(PathBuf::from(value()?)),
            "--show-collision" => options.show_collision = true,
            "--plants" => options.plants = true,
            "--only" => options.only = true,
            "--species" => {
                options.plants = true;
                options.species.push(PathBuf::from(value()?));
            }
            "--grass" => {
                options.plants = true;
                options.grass.push(PathBuf::from(value()?));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if options.only && !options.plants {
        return Err(
            "--only keeps just the species and grass files given: it needs --plants, --species or --grass"
                .to_owned(),
        );
    }
    Ok(options)
}

fn load_scene(options: &Options) -> Result<SceneAsset, String> {
    if options.forest {
        return Ok(forest::build(options.scene_texture_size.unwrap_or(1024)));
    }
    if options.plants {
        return plants::build(
            options.scene_texture_size.unwrap_or(1024),
            &options.species,
            &options.grass,
            options.only,
        );
    }
    if options.gallery {
        return Ok(gallery::build(options.scene_texture_size.unwrap_or(1024)));
    }
    if let Some(path) = options
        .scene_file
        .as_ref()
        .or(options.project_scene.as_ref())
    {
        let (scene, mut warnings) = if options.scene_file.is_some() {
            scene_file::build(path, options.scene_texture_size)?
        } else {
            project::build(path, options.scene_texture_size)?
        };
        warnings.sort();
        warnings.dedup();
        for warning in warnings {
            println!("warning: {warning}");
        }
        return Ok(scene);
    }
    let Some(path) = &options.gltf else {
        return Ok(scene::build_temple(options.quality));
    };
    println!("importing {}...", path.display());
    let imported = import_gltf(path, &options.import).map_err(|e| e.to_string())?;
    for warning in &imported.warnings {
        println!("warning: {warning}");
    }
    Ok(imported.scene)
}

/// Bakes a scene too large to hold in memory.
fn bake_large(path: &std::path::Path, options: &Options, start: Instant) -> Result<(), String> {
    println!("baking {} from disk...", path.display());
    let mut report = solstice_import::streamed::bake_large(path, &options.output, &options.large)?;
    report.warnings.sort();
    report.warnings.dedup();
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    println!(
        "{} meshes, {} instances, {} unique triangles, {} instanced triangles",
        report.meshes, report.instances, report.unique_triangles, report.instanced_triangles,
    );
    println!("{} glow maps, {} cards", report.glow_maps, report.cards);
    println!(
        "{} clusters, {} pages, {} texture layers; building {:.1}s, paging {:.1}s, textures {:.1}s",
        report.clusters,
        report.pages,
        report.texture_layers,
        report.build_seconds,
        report.page_seconds,
        report.texture_seconds,
    );
    println!(
        "wrote {} ({:.1} MB) in {:.1}s",
        options.output.display(),
        report.file_bytes as f64 / 1.0e6,
        start.elapsed().as_secs_f32()
    );
    Ok(())
}

/// Checks a character file, and fails if anything is wrong with it.
fn check_character(path: &std::path::Path, roles: Option<&PathBuf>) -> Result<(), String> {
    use solstice_import::character::{check_character, Roles};
    let roles = match roles {
        Some(file) => {
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            Some(Roles::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?)
        }
        None => None,
    };
    let report = check_character(path, roles.as_ref())?;
    println!("{}: {}", path.display(), report.summary());
    if report.problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{} is not ready for the engine", path.display()))
    }
}

/// The surfaces a scene's land is laid with: its bands, and the plain terrain's sand.
const LAND_SURFACES: [&str; 3] = ["grass", "sand", "dirt"];

/// Writes each of the land's surfaces, compressed, as a scene file of one layer.
fn write_surfaces(folder: &Path, size: u32) -> Result<(), String> {
    use solstice_bake::textures;
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    for name in LAND_SURFACES {
        let start = Instant::now();
        let bytes = textures::encode_layer(textures::surface(name, size), size);
        let path = folder.join(format!(
            "{}-{size}.solstice",
            textures::surface_key(name, 0)
        ));
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!(
            "wrote {} ({:.1} MB) in {:.1}s",
            path.display(),
            bytes.len() as f64 / 1e6,
            start.elapsed().as_secs_f32()
        );
    }
    Ok(())
}

/// The file of what a game's scenes share, beside theirs (the viewer's `shared.rs`).
const SHARED_FILE: &str = "shared.solstice";

/// Whether a scene has actors whose models are in the shared file: it is read with it.
fn reads_shared(scene: &solstice_asset::SceneAsset) -> bool {
    scene
        .gameplay
        .iter()
        .filter(|file| file.name == solstice_game::GAME_FILE)
        .flat_map(|file| file.text.lines())
        .any(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.split_first() {
                Some((&"actor", rest)) => {
                    solstice_game::scene::ActorLine::parse(rest).is_ok_and(|actor| actor.shared)
                }
                _ => false,
            }
        })
}

/// What a baked scene lends, as the text of its list: nothing, if it has no list.
fn lender(path: &Path) -> Result<String, String> {
    use solstice_asset::file;
    use std::io::{Read, Seek, SeekFrom};
    let failed = |error: std::io::Error| error.to_string();
    let mut scene = std::fs::File::open(path).map_err(failed)?;
    let mut head = [0; file::HEADER_BYTES];
    scene.read_exact(&mut head).map_err(failed)?;
    let header = file::decode_header(&head).map_err(|error| error.to_string())?;
    scene
        .seek(SeekFrom::Start(header.manifest_offset))
        .map_err(failed)?;
    let mut packed = vec![0; header.manifest_bytes as usize];
    scene.read_exact(&mut packed).map_err(failed)?;
    let manifest = file::inflate(&packed).map_err(|error| error.to_string())?;
    let scene = file::decode_manifest(&manifest, header.manifest_offset + header.manifest_bytes)
        .map_err(|error| error.to_string())?;
    Ok(scene
        .gameplay
        .into_iter()
        .find(|file| file.name == solstice_asset::join::LENDS)
        .map(|file| file.text)
        .unwrap_or_default())
}

fn main() -> Result<(), String> {
    let mut options = parse_args()?;
    if options.scene_file.is_some() && options.project_scene.is_some() {
        return Err("give --scene or --project, not both".into());
    }
    if let Some(folder) = &options.surfaces {
        return write_surfaces(folder, options.scene_texture_size.unwrap_or(1024));
    }
    if let Some(path) = &options.convert {
        let written = project::convert_file(path)?;
        println!("wrote {}", written.display());
        return Ok(());
    }
    if let (Some(scene), false) = (&options.project_scene, options.output_given) {
        let stem = scene.file_stem().ok_or("--project takes a .scene file")?;
        options.output = project::project_folder(scene)?
            .join("build")
            .join(stem)
            .with_extension("solstice");
    }
    if let Some(path) = &options.check_character {
        return check_character(path, options.roles.as_ref());
    }
    // A truck built of pieces is packed, each piece to be read alone (`kit.rs`).
    if let Some(actor) = &options.kit {
        return kit::bake_kit(actor, &options.output);
    }
    let start = Instant::now();
    if let Some(path) = &options.web_split {
        return web_split(path, &options.output).map_err(|e| e.to_string());
    }
    if let Some(path) = &options.large_gltf {
        return bake_large(path, &options, start);
    }
    let mut scene = load_scene(&options)?;
    if options.show_collision {
        scene = collision_view::collision_scene(&scene)?;
    }
    // A scene read with the file of what its game's maps share leaves out of its own what
    // that file lends: the file is beside where this one is written.
    if std::env::var_os("SOLSTICE_WHOLE").is_none() && reads_shared(&scene) {
        let beside = options.output.with_file_name(SHARED_FILE);
        let lends = lender(&beside).map_err(|error| {
            format!(
                "{}: {error}: this scene is read with it, so bake it first",
                beside.display()
            )
        })?;
        let left = solstice_bake::lend::borrow(&mut scene, &lends);
        println!(
            "left to {}: {} meshes, {} texture layers",
            beside.display(),
            left.meshes,
            left.textures
        );
        if left.meshes + left.textures > 0 {
            scene.gameplay.push(solstice_asset::GameFile {
                name: solstice_asset::join::BORROWS.to_owned(),
                text: left.text,
            });
        }
    }
    if !options.raw_textures {
        println!("compressing {} texture layers...", scene.textures.len());
        solstice_builder::compress_textures(&mut scene);
    }

    println!(
        "{} meshes, {} instances, {} unique triangles, {} instanced triangles",
        scene.meshes.len(),
        scene.instances.len(),
        scene.unique_source_triangles(),
        scene.total_source_triangles(),
    );

    let bytes = solstice_asset::file::encode(&solstice_builder::paginate(&scene));
    if let Some(parent) = options.output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&options.output, &bytes).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({:.1} MB) in {:.1}s",
        options.output.display(),
        bytes.len() as f64 / 1.0e6,
        start.elapsed().as_secs_f32()
    );
    Ok(())
}
