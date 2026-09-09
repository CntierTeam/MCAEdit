use mcaedit_core::blockstate::BlockState;
use mcaedit_core::session::Session;
use mcaedit_core::world::WorldView;
use std::fs;
use tempfile::TempDir;

fn make_world(tmp: &TempDir) -> std::path::PathBuf {
    let world = tmp.path().join("world");
    fs::create_dir_all(world.join("region")).unwrap();
    fs::create_dir_all(world.join("entities")).unwrap();
    world
}

fn with_stack<F: FnOnce() + Send + 'static>(f: F) {
    let builder = std::thread::Builder::new()
        .name("mcaedit-ops-we".into())
        .stack_size(32 * 1024 * 1024);
    builder.spawn(f).expect("spawn").join().expect("join");
}

#[test]
fn replace_walls_sphere_stack_clipboard() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("we".into()), None).unwrap();
        let mut session = Session::open(&cwd, "we").unwrap();
        let mut wv = WorldView::new(&mut session);

        wv.fill(
            0,
            64,
            0,
            4,
            66,
            4,
            BlockState::parse("minecraft:dirt").unwrap(),
        )
        .unwrap();
        wv.replace(
            0,
            64,
            0,
            4,
            66,
            4,
            BlockState::air(),
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        // dirt interior still dirt; we replaced air only — box was solid dirt so 0 air
        assert_eq!(
            wv.get_block(2, 65, 2).unwrap().to_compact(),
            "minecraft:dirt"
        );

        wv.replace(
            0,
            64,
            0,
            4,
            66,
            4,
            BlockState::parse("minecraft:dirt").unwrap(),
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        assert_eq!(
            wv.get_block(2, 65, 2).unwrap().to_compact(),
            "minecraft:stone"
        );

        wv.walls(
            10,
            64,
            10,
            14,
            66,
            14,
            BlockState::parse("minecraft:oak_planks").unwrap(),
        )
        .unwrap();
        assert_eq!(
            wv.get_block(10, 65, 12).unwrap().to_compact(),
            "minecraft:oak_planks"
        );
        assert!(wv.get_block(12, 65, 12).unwrap().is_air_like());

        wv.sphere(
            20,
            70,
            20,
            3.0,
            BlockState::parse("minecraft:glass").unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            wv.get_block(20, 70, 20).unwrap().to_compact(),
            "minecraft:glass"
        );

        wv.fill(
            30,
            64,
            30,
            31,
            64,
            30,
            BlockState::parse("minecraft:gold_block").unwrap(),
        )
        .unwrap();
        wv.stack(30, 64, 30, 31, 64, 30, 2, 3, 0, 0).unwrap();
        assert_eq!(
            wv.get_block(33, 64, 30).unwrap().to_compact(),
            "minecraft:gold_block"
        );
        assert_eq!(
            wv.get_block(36, 64, 30).unwrap().to_compact(),
            "minecraft:gold_block"
        );

        wv.clipboard_copy(30, 64, 30, 31, 64, 30).unwrap();
        wv.clipboard_rotate_yaw(90).unwrap();
        wv.clipboard_paste(40, 64, 40).unwrap();
        // 2x1x1 rotated 90 → 1x1x2
        assert_eq!(
            wv.get_block(40, 64, 40).unwrap().to_compact(),
            "minecraft:gold_block"
        );
        assert_eq!(
            wv.get_block(40, 64, 41).unwrap().to_compact(),
            "minecraft:gold_block"
        );

        wv.hollow(10, 64, 10, 14, 66, 14).unwrap();
        // walls remain, interior still air
        assert_eq!(
            wv.get_block(10, 65, 12).unwrap().to_compact(),
            "minecraft:oak_planks"
        );
    });
}
