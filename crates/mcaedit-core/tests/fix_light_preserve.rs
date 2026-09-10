//! Regression: `edit fix-light` must preserve block palettes (not regenerate).

use mcaedit_core::blockstate::BlockState;
use mcaedit_core::commit::commit_session;
use mcaedit_core::session::Session;
use mcaedit_core::world::WorldView;
use std::fs;
use tempfile::TempDir;

fn with_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .name("fix-light-preserve".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join");
}

#[test]
fn fix_light_preserves_edited_blocks() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = tmp.path().join("world");
        fs::create_dir_all(world.join("region")).unwrap();
        fs::create_dir_all(world.join("entities")).unwrap();
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("fixlight".into()), None).unwrap();
        let mut session = Session::open(&cwd, "fixlight").unwrap();
        {
            let mut wv = WorldView::new(&mut session);

            wv.fill(
                -8,
                60,
                -8,
                8,
                62,
                8,
                BlockState::parse("minecraft:dirt").unwrap(),
            )
            .unwrap();
            wv.fill(
                -8,
                63,
                -8,
                8,
                63,
                8,
                BlockState::parse("minecraft:grass_block").unwrap(),
            )
            .unwrap();
            wv.fill(
                0,
                64,
                0,
                4,
                67,
                4,
                BlockState::parse("minecraft:oak_planks").unwrap(),
            )
            .unwrap();
            wv.fill(
                1,
                65,
                1,
                3,
                66,
                3,
                BlockState::parse("minecraft:air").unwrap(),
            )
            .unwrap();

            assert_eq!(
                wv.get_block(0, 64, 0).unwrap().to_compact(),
                "minecraft:oak_planks"
            );
            assert_eq!(
                wv.get_block(2, 65, 2).unwrap().to_compact(),
                "minecraft:air"
            );
            assert!(wv.get_block(0, 63, 0).unwrap().to_compact().contains("grass"));

            wv.fix_light(-1, -1, 1, 1, 0, "overworld").unwrap();

            assert_eq!(
                wv.get_block(0, 64, 0).unwrap().to_compact(),
                "minecraft:oak_planks",
                "fix-light must not move/destroy oak_planks"
            );
            assert_eq!(
                wv.get_block(4, 67, 4).unwrap().to_compact(),
                "minecraft:oak_planks"
            );
            assert_eq!(
                wv.get_block(2, 65, 2).unwrap().to_compact(),
                "minecraft:air"
            );
            assert!(
                wv.get_block(0, 63, 0).unwrap().to_compact().contains("grass"),
                "ground must survive fix-light"
            );
            assert_eq!(
                wv.get_block(0, 0, 0).unwrap().to_compact(),
                "minecraft:air",
                "blocks must not shift down to y=0"
            );
        }

        commit_session(&cwd, &mut session, false).unwrap();

        // Fresh session synced from committed source world
        Session::create(
            &cwd,
            &world,
            "overworld",
            Some("fixlight2".into()),
            None,
        )
        .unwrap();
        let mut session2 = Session::open(&cwd, "fixlight2").unwrap();
        let wv2 = WorldView::new(&mut session2);
        assert_eq!(
            wv2.get_block(0, 64, 0).unwrap().to_compact(),
            "minecraft:oak_planks",
            "committed world must keep oak_planks after fix-light"
        );
    });
}
