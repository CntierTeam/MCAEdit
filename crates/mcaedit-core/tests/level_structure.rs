//! level.dat + structure + version regression tests.

use mcaedit_core::blockstate::BlockState;
use mcaedit_core::level::{self, GeneratorKind, LevelPatchOptions, WorldCreateOptions};
use mcaedit_core::mc_version::ResolvedVersion;
use mcaedit_core::session::{CreateSessionOpts, Session};
use mcaedit_core::structure::{self, PlaceOptions};
use mcaedit_core::world::WorldView;
use std::fs;
use tempfile::TempDir;

#[test]
fn level_dat_roundtrip_two_dataversions() {
    let tmp = TempDir::new().unwrap();
    for (mc, dv) in [("26.2", 4903), ("1.18.2", 2975)] {
        let path = tmp.path().join(mc);
        let opts = WorldCreateOptions {
            path: path.clone(),
            level_name: format!("lv-{mc}"),
            seed: 12345,
            spawn: [16, 80, -16],
            game_type: 1,
            generator: GeneratorKind::Noise,
            version: ResolvedVersion::from_mc(mc).unwrap(),
            ..Default::default()
        };
        let info = level::create_world(&opts).unwrap();
        assert_eq!(info.data_version, Some(dv));
        assert_eq!(info.seed, Some(12345));
        assert_eq!(info.level_name, format!("lv-{mc}"));

        let again = level::info(&path).unwrap();
        assert_eq!(again.data_version, Some(dv));
        assert_eq!(again.seed, Some(12345));

        let patched = level::update_level_dat(
            &path,
            &LevelPatchOptions {
                level_name: Some("patched"),
                seed: Some(7),
                spawn: Some([1, 2, 3]),
                game_type: Some(0),
                touch_last_played: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(patched.level_name, "patched");
        assert_eq!(patched.seed, Some(7));
        assert_eq!(patched.spawn, [1, 2, 3]);
        assert_eq!(patched.game_type, Some(0));
        assert_eq!(patched.data_version, Some(dv));
    }
}

#[test]
fn session_bootstrap_empty_world() {
    let tmp = TempDir::new().unwrap();
    let world = tmp.path().join("newworld");
    let cwd = tmp.path().to_path_buf();
    let opts = CreateSessionOpts {
        bootstrap: true,
        force_level: false,
        level_name: Some("boot".into()),
        seed: Some(99),
        version: Some(ResolvedVersion::from_mc("1.21.4").unwrap()),
        generator: Some(GeneratorKind::Flat),
        region_format: Some(level::RegionFormat::Anvil),
        data_version: None,
    };
    let s = Session::create_with_opts(
        &cwd,
        &world,
        "overworld",
        Some("boot1".into()),
        Some("agent".into()),
        opts,
    )
    .unwrap();
    assert!(world.join("level.dat").is_file());
    assert!(world.join("region").is_dir());
    assert_eq!(s.meta.data_version, Some(4189));
    let info = level::info(&world).unwrap();
    assert_eq!(info.seed, Some(99));
    assert_eq!(info.level_name, "boot");
}

#[test]
fn structure_place_undo_and_medium_stress() {
    let builder = std::thread::Builder::new()
        .name("structure-stress".into())
        .stack_size(32 * 1024 * 1024);
    let handle = builder
        .spawn(|| {
            let tmp = TempDir::new().unwrap();
            let world = tmp.path().join("world");
            fs::create_dir_all(world.join("region")).unwrap();
            fs::create_dir_all(world.join("entities")).unwrap();
            let cwd = tmp.path().to_path_buf();
            let mut session =
                Session::create(&cwd, &world, "overworld", Some("st".into()), None).unwrap();
            {
                let mut wv = WorldView::new(&mut session);
                // 16³ box
                wv.fill(
                    0,
                    64,
                    0,
                    15,
                    79,
                    15,
                    BlockState::parse("minecraft:stone").unwrap(),
                )
                .unwrap();
                let out = tmp.path().join("box.nbt");
                structure::export_aabb(&wv, [0, 64, 0], [15, 79, 15], &out, 4903).unwrap();
                let meta = structure::info(&out).unwrap();
                assert_eq!(meta.size, [16, 16, 16]);

                structure::place(
                    &mut wv,
                    &out,
                    64,
                    64,
                    64,
                    PlaceOptions {
                        rotation: 180,
                        mirror: Some('x'),
                        include_entities: true,
                    },
                )
                .unwrap();
                assert_eq!(
                    wv.get_block(64, 64, 64).unwrap().to_compact(),
                    "minecraft:stone"
                );

                // undo place
                let target = session.history.undo_target().unwrap();
                assert!(target.description.contains("structure place"));
            }
            {
                let mut wv = WorldView::new(&mut session);
                wv.undo(1).unwrap();
                assert_eq!(
                    wv.get_block(64, 64, 64).unwrap().to_compact(),
                    "minecraft:air"
                );
            }
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn empty_chunk_preserves_session_data_version() {
    let tmp = TempDir::new().unwrap();
    let world = tmp.path().join("w");
    let opts = WorldCreateOptions {
        path: world.clone(),
        version: ResolvedVersion::from_mc("1.20.1").unwrap(),
        ..Default::default()
    };
    level::create_world(&opts).unwrap();
    let cwd = tmp.path().to_path_buf();
    let mut session = Session::create(&cwd, &world, "overworld", Some("dv".into()), None).unwrap();
    assert_eq!(session.meta.data_version, Some(3465));
    let wv = WorldView::new(&mut session);
    let chunk = wv.load_chunk(5, 5).unwrap();
    assert_eq!(chunk.data_version(), Some(3465));
}
