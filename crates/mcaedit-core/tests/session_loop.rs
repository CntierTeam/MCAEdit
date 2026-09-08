use mcaedit_core::blockstate::BlockState;
use mcaedit_core::palette::{SectionBlocks, SectionDiff};
use mcaedit_core::session::Session;
use mcaedit_core::world::WorldView;
use mcaedit_core::{commit_session, ActionPayload};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn make_world(tmp: &TempDir) -> std::path::PathBuf {
    let world = tmp.path().join("world");
    fs::create_dir_all(world.join("region")).unwrap();
    fs::create_dir_all(world.join("entities")).unwrap();
    world
}

#[test]
fn palette_roundtrip_mixed() {
    let mut sec = SectionBlocks::air();
    sec.set(0, 0, 0, BlockState::parse("minecraft:stone").unwrap());
    sec.set(
        1,
        0,
        0,
        BlockState::parse("minecraft:oak_log[axis=y]").unwrap(),
    );
    let (palette, data) = sec.to_palette_nbt().unwrap();
    assert!(palette.len() >= 2);
    let back = SectionBlocks::from_palette_nbt(&palette, data.as_deref()).unwrap();
    assert_eq!(back.get(0, 0, 0).to_compact(), "minecraft:stone");
    assert_eq!(
        back.get(1, 0, 0).to_compact(),
        "minecraft:oak_log[axis=y]"
    );
}

#[test]
fn session_edit_undo_commit_loop() {
    let builder = std::thread::Builder::new()
        .name("mcaedit-session-loop".into())
        .stack_size(32 * 1024 * 1024);
    let handle = builder
        .spawn(|| {
            let tmp = TempDir::new().unwrap();
            let world = make_world(&tmp);
            let cwd = tmp.path().to_path_buf();

            let session = Session::create(&cwd, &world, "overworld", Some("t1".into()), None).unwrap();
            assert_eq!(session.meta.id, "t1");

            let mut session = Session::open(&cwd, "t1").unwrap();
            {
                let mut wv = WorldView::new(&mut session);
                let a = wv
                    .set_block(1, 64, 2, BlockState::parse("minecraft:stone").unwrap())
                    .unwrap();
                assert_eq!(a.id, 1);
                assert_eq!(
                    wv.get_block(1, 64, 2).unwrap().to_compact(),
                    "minecraft:stone"
                );

                let a2 = wv
                    .fill(
                        0,
                        64,
                        0,
                        2,
                        64,
                        2,
                        BlockState::parse("minecraft:dirt").unwrap(),
                    )
                    .unwrap();
                assert!(a2.changed_count() > 0);
                assert_eq!(
                    wv.get_block(1, 64, 2).unwrap().to_compact(),
                    "minecraft:dirt"
                );

                let mut diff = SectionDiff::default();
                diff.set(3, 0, 3, BlockState::parse("minecraft:gold_block").unwrap());
                diff.set(4, 0, 4, BlockState::void_air());
                let a3 = wv.set_section(0, 4, 0, diff).unwrap();
                assert!(matches!(a3.payload, ActionPayload::SetSection { .. }));
                assert_eq!(
                    wv.get_block(3, 64, 3).unwrap().to_compact(),
                    "minecraft:gold_block"
                );

                let ent = json!({
                    "id": "minecraft:armor_stand",
                    "Pos": [1.5, 65.0, 2.5]
                });
                let a4 = wv.entity_spawn(ent).unwrap();
                assert!(matches!(a4.payload, ActionPayload::EntitySpawn { .. }));
                let ents = wv.entity_store().list_near(1.5, 65.0, 2.5, 8.0).unwrap();
                assert_eq!(ents.len(), 1);

                wv.undo(1).unwrap();
                let ents = wv.entity_store().list_near(1.5, 65.0, 2.5, 8.0).unwrap();
                assert!(ents.is_empty());

                wv.redo(1).unwrap();
                let ents = wv.entity_store().list_near(1.5, 65.0, 2.5, 8.0).unwrap();
                assert_eq!(ents.len(), 1);

                let tpl = mcaedit_core::Template::capture(&wv, "pad", 0, 64, 0, 2, 65, 2).unwrap();
                let path = tpl.save_to_disk(&cwd).unwrap();
                assert!(path.exists());
            }

            let report = commit_session(&cwd, &mut session, false).unwrap();
            assert!(report.iter().any(|l| l.contains("region/")));
            assert!(!session.meta.dirty);
            assert!(world.join("region/r.0.0.mca").exists());

            // second session on same world (collaboration)
            let s2 = Session::create(
                &cwd,
                &world,
                "overworld",
                Some("t2".into()),
                Some("agent-b".into()),
            )
            .unwrap();
            assert_eq!(s2.meta.label.as_deref(), Some("agent-b"));
            let listed = Session::list(&cwd).unwrap();
            assert!(listed.len() >= 2);
        })
        .expect("spawn");
    handle.join().expect("thread panicked");
}
