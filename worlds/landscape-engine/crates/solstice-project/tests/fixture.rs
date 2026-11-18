//! The fixture holds a scene with every kind of thing in it, as this crate writes it. The editor's
//! own writer (`apps/viewer/web/editor/project.js`) is held to the same file by
//! `apps/viewer/web/editor/test.mjs`, so a scene saved by either is byte for byte the same.

use solstice_project::Scene;

#[test]
fn the_fixture_reads_and_writes_back_unchanged() {
    let text = include_str!("fixture.scene").replace("\r\n", "\n");
    let scene = Scene::parse(&text).unwrap();
    assert_eq!(scene.to_json(), text);
    let kinds: Vec<&str> = scene.walk().iter().flat_map(|e| e.kinds()).collect();
    for kind in [
        "mesh", "actor", "light", "blocker", "wood", "clear", "path", "yard", "pond",
    ] {
        assert!(kinds.contains(&kind), "the fixture has no {kind}");
    }
}
