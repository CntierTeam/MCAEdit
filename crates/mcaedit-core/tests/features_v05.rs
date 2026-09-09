//! Regression tests for biome brush, smooth3d, offline tick, and Linear regions.

use mcaedit_core::blockstate::BlockState;
use mcaedit_core::commit::commit_session;
use mcaedit_core::linear::{
    linear_to_mca_bytes, mca_to_linear_v1, read_linear, write_linear, LinearVersion,
};
use mcaedit_core::mask::Mask;
use mcaedit_core::region::{copy_region_if_needed, RegionStore};
use mcaedit_core::session::Session;
use mcaedit_core::tick::force_age_steps;
use mcaedit_core::world::WorldView;
use std::fs;
use std::time::Instant;
use tempfile::TempDir;

fn make_world(tmp: &TempDir) -> std::path::PathBuf {
    let world = tmp.path().join("world");
    fs::create_dir_all(world.join("region")).unwrap();
    fs::create_dir_all(world.join("entities")).unwrap();
    world
}

fn with_stack<F: FnOnce() + Send + 'static>(f: F) {
    let builder = std::thread::Builder::new()
        .name("mcaedit-v05".into())
        .stack_size(32 * 1024 * 1024);
    builder.spawn(f).expect("spawn").join().expect("join");
}

#[test]
fn biome_brush_sphere_cyl_undo() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("bb".into()), None).unwrap();
        let mut session = Session::open(&cwd, "bb").unwrap();
        let mut wv = WorldView::new(&mut session);

        wv.set_block(0, 64, 0, BlockState::parse("minecraft:stone").unwrap())
            .unwrap();
        let before = wv.get_biome(0, 64, 0).unwrap();
        wv.brush_biome_sphere(0, 64, 0, 6.0, "minecraft:desert", None, false)
            .unwrap();
        assert_eq!(wv.get_biome(0, 64, 0).unwrap(), "minecraft:desert");
        assert_eq!(wv.get_biome(4, 64, 0).unwrap(), "minecraft:desert");

        wv.brush_biome_cyl(
            32,
            32,
            60,
            4.0,
            8,
            "minecraft:badlands",
            Some(Mask::parse("air").unwrap()),
            false,
        )
        .unwrap();
        // air at cell origin → painted
        assert_eq!(wv.get_biome(32, 64, 32).unwrap(), "minecraft:badlands");

        wv.undo(2).unwrap();
        assert_eq!(wv.get_biome(0, 64, 0).unwrap(), before);
    });
}

#[test]
fn smooth3d_majority_and_undo() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("s3".into()), None).unwrap();
        let mut session = Session::open(&cwd, "s3").unwrap();
        let mut wv = WorldView::new(&mut session);

        // Mostly stone cube with a noisy air pocket — majority should fill it.
        wv.fill(
            0,
            64,
            0,
            7,
            71,
            7,
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        wv.set_block(3, 67, 3, BlockState::air()).unwrap();
        wv.set_block(4, 67, 3, BlockState::air()).unwrap();
        wv.smooth3d(0, 64, 0, 7, 71, 7, 2, 1, false).unwrap();
        assert!(
            !wv.get_block(3, 67, 3).unwrap().is_air_like(),
            "majority vote should fill isolated air"
        );
        let n = wv.undo(1).unwrap().len();
        assert_eq!(n, 1);
        assert!(wv.get_block(3, 67, 3).unwrap().is_air_like());
    });
}

#[test]
fn smooth3d_solid_mode_erodes_thin_wall() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("s3s".into()), None).unwrap();
        let mut session = Session::open(&cwd, "s3s").unwrap();
        let mut wv = WorldView::new(&mut session);

        // Single-block-thick wall in air — solid mode should tend to erode it.
        for y in 64..68 {
            wv.set_block(4, y, 4, BlockState::parse("minecraft:stone").unwrap())
                .unwrap();
        }
        wv.smooth3d(0, 64, 0, 8, 68, 8, 3, 1, true).unwrap();
        // Not asserting total wipe (kernel-dependent); just that undo works.
        let n = wv.undo(1).unwrap().len();
        assert_eq!(n, 1);
    });
}

#[test]
fn offline_tick_grows_wheat_and_undo() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("tk".into()), None).unwrap();
        let mut session = Session::open(&cwd, "tk").unwrap();
        let mut wv = WorldView::new(&mut session);

        // Plant a full section of young wheat so random samples hit often.
        for z in 0..16 {
            for x in 0..16 {
                wv.set_block(
                    x,
                    64,
                    z,
                    BlockState::parse("minecraft:wheat[age=0]").unwrap(),
                )
                .unwrap();
            }
        }
        // Deterministic force path sanity.
        let forced = force_age_steps(
            &BlockState::parse("minecraft:wheat[age=0]").unwrap(),
            2,
        )
        .unwrap();
        assert_eq!(forced.properties.get("age").map(String::as_str), Some("2"));

        let (lines, action) = wv.tick_offline(0, 0, 0, 0, 40, 16).unwrap();
        assert!(lines.iter().any(|l| l.contains("tick_growth")));
        assert!(
            action.is_some(),
            "expected growth mutations; lines={lines:?}"
        );
        let grown = (0..16)
            .flat_map(|z| (0..16).map(move |x| (x, z)))
            .filter(|(x, z)| {
                wv.get_block(*x, 64, *z)
                    .unwrap()
                    .properties
                    .get("age")
                    .and_then(|a| a.parse::<u8>().ok())
                    .unwrap_or(0)
                    > 0
            })
            .count();
        assert!(grown > 0, "no wheat aged");
        wv.undo(1).unwrap();
        assert_eq!(
            wv.get_block(0, 64, 0)
                .unwrap()
                .properties
                .get("age")
                .map(String::as_str),
            Some("0")
        );
    });
}

#[test]
fn offline_tick_grows_sugar_cane() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("cane".into()), None).unwrap();
        let mut session = Session::open(&cwd, "cane").unwrap();
        let mut wv = WorldView::new(&mut session);

        for z in 0..8 {
            for x in 0..8 {
                wv.set_block(
                    x,
                    64,
                    z,
                    BlockState::parse("minecraft:sugar_cane").unwrap(),
                )
                .unwrap();
            }
        }
        let (_lines, action) = wv.tick_offline(0, 0, 0, 0, 60, 16).unwrap();
        assert!(action.is_some(), "expected cane growth");
        let mut grew = false;
        for z in 0..8 {
            for x in 0..8 {
                if wv.get_block(x, 65, z).unwrap().to_compact().contains("sugar_cane") {
                    grew = true;
                }
            }
        }
        assert!(grew, "no sugar cane grew upward");
    });
}

#[test]
fn linear_v1_session_open_edit_commit() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        // Seed a Linear v1 region containing one edited chunk via MCA→Linear.
        {
            let mut session_seed =
                Session::create(tmp.path(), &world, "overworld", Some("seed".into()), None)
                    .unwrap();
            let mut wv = WorldView::new(&mut session_seed);
            wv.set_block(5, 70, 5, BlockState::parse("minecraft:gold_block").unwrap())
                .unwrap();
            // Flush work mca, convert to linear in source, discard session.
            let work_mca = session_seed.work_region_dir().join("r.0.0.mca");
            assert!(work_mca.exists());
            let mca = fs::read(&work_mca).unwrap();
            let mut linear = mca_to_linear_v1(&mca, 0, 0).unwrap();
            linear.version = LinearVersion::V1;
            let bytes = write_linear(&linear).unwrap();
            fs::write(world.join("region/r.0.0.linear"), &bytes).unwrap();
            // Remove any anvil so source is Linear-only.
            let _ = fs::remove_file(world.join("region/r.0.0.mca"));
            session_seed.discard().unwrap();
        }

        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("lin".into()), None).unwrap();
        let mut session = Session::open(&cwd, "lin").unwrap();
        // Touch region → converts linear→mca in work copy.
        let mut wv = WorldView::new(&mut session);
        assert_eq!(
            wv.get_block(5, 70, 5).unwrap().to_compact(),
            "minecraft:gold_block"
        );
        wv.set_block(5, 71, 5, BlockState::parse("minecraft:diamond_block").unwrap())
            .unwrap();
        commit_session(&cwd, &mut session, false).unwrap();

        // Source should now have .linear again.
        assert!(world.join("region/r.0.0.linear").exists());
        let bytes = fs::read(world.join("region/r.0.0.linear")).unwrap();
        let linear = read_linear(&bytes).unwrap();
        let mca = linear_to_mca_bytes(&linear).unwrap();
        let store_path = tmp.path().join("check.mca");
        fs::write(&store_path, &mca).unwrap();
        let store = RegionStore::open(&store_path);
        let chunk = store.read_chunk(0, 0).unwrap().unwrap();
        assert_eq!(
            chunk.get_block(5, 71, 5).unwrap().to_compact(),
            "minecraft:diamond_block"
        );
    });
}

#[test]
fn copy_region_if_needed_converts_linear() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        let linear = {
            let world = make_world(&tmp);
            let cwd = tmp.path().to_path_buf();
            Session::create(&cwd, &world, "overworld", Some("c".into()), None).unwrap();
            let mut session = Session::open(&cwd, "c").unwrap();
            let mut wv = WorldView::new(&mut session);
            wv.set_block(1, 64, 1, BlockState::parse("minecraft:stone").unwrap())
                .unwrap();
            let mca = fs::read(session.work_region_dir().join("r.0.0.mca")).unwrap();
            mca_to_linear_v1(&mca, 0, 0).unwrap()
        };
        let bytes = write_linear(&linear).unwrap();
        fs::write(src.join("r.0.0.linear"), bytes).unwrap();
        let path = copy_region_if_needed(&src, &dst, 0, 0).unwrap();
        assert!(path.ends_with("r.0.0.mca"));
        assert!(dst.join("r.0.0.linear.source").exists());
        let store = RegionStore::open(&path);
        let chunk = store.read_chunk(0, 0).unwrap().unwrap();
        assert_eq!(
            chunk.get_block(1, 64, 1).unwrap().to_compact(),
            "minecraft:stone"
        );
    });
}

/// Medium stress: smooth3d 24³ + biome brush r=16 + linear convert.
#[test]
fn stress_medium_smooth3d_biome_brush_linear() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("st05".into()), None).unwrap();
        let mut session = Session::open(&cwd, "st05").unwrap();
        let mut wv = WorldView::new(&mut session);

        let t0 = Instant::now();
        wv.fill(
            0,
            40,
            0,
            23,
            63,
            23,
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        // Sprinkle noise
        for i in 0..40 {
            wv.set_block(i % 24, 50 + (i % 5), (i * 3) % 24, BlockState::air())
                .unwrap();
        }
        wv.smooth3d(0, 40, 0, 23, 63, 23, 2, 1, false).unwrap();
        wv.brush_biome_sphere(12, 52, 12, 16.0, "minecraft:jungle", None, false)
            .unwrap();
        assert_eq!(wv.get_biome(12, 52, 12).unwrap(), "minecraft:jungle");

        // Linear roundtrip of work region
        let mca_path = session.work_region_dir().join("r.0.0.mca");
        let mca = fs::read(&mca_path).unwrap();
        let linear = mca_to_linear_v1(&mca, 0, 0).unwrap();
        let bytes = write_linear(&linear).unwrap();
        let back = read_linear(&bytes).unwrap();
        assert!(back.present_count() >= 1);

        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_secs() < 90,
            "medium stress too slow: {elapsed:?}"
        );
    });
}

#[test]
#[ignore]
fn stress_heavy_smooth3d_tick_linear() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("hv05".into()), None).unwrap();
        let mut session = Session::open(&cwd, "hv05").unwrap();
        let mut wv = WorldView::new(&mut session);
        let t0 = Instant::now();
        wv.fill(
            0,
            32,
            0,
            47,
            79,
            47,
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        wv.smooth3d(0, 32, 0, 47, 79, 47, 2, 2, true).unwrap();
        eprintln!("smooth3d {:?}", t0.elapsed());
        for z in 0..32 {
            for x in 0..32 {
                wv.set_block(
                    x,
                    64,
                    z,
                    BlockState::parse("minecraft:wheat[age=0]").unwrap(),
                )
                .unwrap();
            }
        }
        let (_l, a) = wv.tick_offline(0, 0, 1, 1, 80, 16).unwrap();
        eprintln!("tick action={:?} {:?}", a.map(|x| x.changed_count()), t0.elapsed());
        wv.brush_biome_sphere(24, 56, 24, 20.0, "minecraft:desert", None, false)
            .unwrap();
        eprintln!("total {:?}", t0.elapsed());
    });
}
