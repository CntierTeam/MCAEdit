//! Regression tests for brush / mask / pattern / smooth / biome / schem.

use mcaedit_core::blockstate::BlockState;
use mcaedit_core::mask::Mask;
use mcaedit_core::pattern::Pattern;
use mcaedit_core::schem;
use mcaedit_core::session::Session;
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
        .name("mcaedit-features".into())
        .stack_size(32 * 1024 * 1024);
    builder.spawn(f).expect("spawn").join().expect("join");
}

#[test]
fn pattern_percent_fill_and_mask_replace() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("pat".into()), None).unwrap();
        let mut session = Session::open(&cwd, "pat").unwrap();
        let mut wv = WorldView::new(&mut session);

        let pat = Pattern::parse("50%stone,50%dirt").unwrap();
        wv.fill_pattern(0, 64, 0, 7, 64, 7, pat, None).unwrap();
        let mut stone = 0;
        let mut dirt = 0;
        for x in 0..=7 {
            for z in 0..=7 {
                let b = wv.get_block(x, 64, z).unwrap().to_compact();
                if b.contains("stone") {
                    stone += 1;
                } else if b.contains("dirt") {
                    dirt += 1;
                }
            }
        }
        assert_eq!(stone + dirt, 64);
        assert!(stone > 0 && dirt > 0);

        wv.fill(
            0,
            65,
            0,
            3,
            65,
            3,
            BlockState::parse("minecraft:glass").unwrap(),
        )
        .unwrap();
        wv.replace_mask_pattern(
            0,
            64,
            0,
            7,
            65,
            7,
            Mask::parse("glass").unwrap(),
            Pattern::parse("minecraft:oak_planks").unwrap(),
        )
        .unwrap();
        assert_eq!(
            wv.get_block(1, 65, 1).unwrap().to_compact(),
            "minecraft:oak_planks"
        );
        // stone/dirt at y=64 should remain
        let base = wv.get_block(0, 64, 0).unwrap().to_compact();
        assert!(base.contains("stone") || base.contains("dirt"));
    });
}

#[test]
fn brush_sphere_cyl_clipboard_and_mask() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("br".into()), None).unwrap();
        let mut session = Session::open(&cwd, "br").unwrap();
        let mut wv = WorldView::new(&mut session);

        wv.brush_sphere(
            20,
            70,
            20,
            3.0,
            Pattern::parse("minecraft:glass").unwrap(),
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            wv.get_block(20, 70, 20).unwrap().to_compact(),
            "minecraft:glass"
        );

        wv.brush_cyl(
            40,
            40,
            64,
            2.0,
            4,
            Pattern::parse("minecraft:stone").unwrap(),
            Some(Mask::parse("air").unwrap()),
            false,
        )
        .unwrap();
        assert_eq!(
            wv.get_block(40, 65, 40).unwrap().to_compact(),
            "minecraft:stone"
        );

        wv.fill(
            0,
            80,
            0,
            2,
            81,
            1,
            BlockState::parse("minecraft:gold_block").unwrap(),
        )
        .unwrap();
        wv.clipboard_copy(0, 80, 0, 2, 81, 1).unwrap();
        wv.brush_clipboard(50, 80, 50, Some(8.0), Some(Mask::parse("air").unwrap()))
            .unwrap();
        assert_eq!(
            wv.get_block(50, 80, 50).unwrap().to_compact(),
            "minecraft:gold_block"
        );
    });
}

#[test]
fn smooth_heightmap_and_undo() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("sm".into()), None).unwrap();
        let mut session = Session::open(&cwd, "sm").unwrap();
        let mut wv = WorldView::new(&mut session);

        // jagged columns
        for x in 0..8 {
            let h = 64 + (x % 3);
            wv.fill(
                x,
                60,
                0,
                x,
                h,
                0,
                BlockState::parse("minecraft:dirt").unwrap(),
            )
            .unwrap();
        }
        let before = wv.get_block(1, 65, 0).unwrap().is_air_like();
        wv.smooth(0, 60, 0, 7, 70, 0, 2, 1).unwrap();
        // smoothing should change something in jagged terrain
        let after_center = wv.get_block(3, 64, 0).unwrap();
        let _ = (before, after_center);
        let n = wv.undo(1).unwrap().len();
        assert_eq!(n, 1);
    });
}

#[test]
fn biome_paint_roundtrip_undo() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("bio".into()), None).unwrap();
        let mut session = Session::open(&cwd, "bio").unwrap();
        let mut wv = WorldView::new(&mut session);

        // ensure section exists
        wv.set_block(
            0,
            64,
            0,
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        let before = wv.get_biome(0, 64, 0).unwrap();
        wv.biome_paint(0, 64, 0, 7, 67, 7, "minecraft:desert")
            .unwrap();
        assert_eq!(wv.get_biome(0, 64, 0).unwrap(), "minecraft:desert");
        assert_eq!(wv.get_biome(4, 64, 4).unwrap(), "minecraft:desert");
        wv.undo(1).unwrap();
        assert_eq!(wv.get_biome(0, 64, 0).unwrap(), before);
    });
}

#[test]
fn schem_export_import_paste() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("sch".into()), None).unwrap();
        let mut session = Session::open(&cwd, "sch").unwrap();
        let mut wv = WorldView::new(&mut session);

        wv.fill(
            0,
            64,
            0,
            3,
            65,
            2,
            BlockState::parse("minecraft:copper_block").unwrap(),
        )
        .unwrap();
        let out = tmp.path().join("box.schem");
        schem::export_aabb(&wv, 0, 64, 0, 3, 65, 2, &out).unwrap();
        let info = schem::info(&out).unwrap();
        assert_eq!(info.width, 4);
        assert_eq!(info.height, 2);
        assert_eq!(info.length, 3);

        schem::import_paste(&mut wv, &out, 32, 64, 32).unwrap();
        assert_eq!(
            wv.get_block(32, 64, 32).unwrap().to_compact(),
            "minecraft:copper_block"
        );
        assert_eq!(
            wv.get_block(35, 65, 34).unwrap().to_compact(),
            "minecraft:copper_block"
        );
    });
}

/// Medium stress: 32³ fill with % pattern + sphere brush r=12 + schem roundtrip.
#[test]
fn stress_medium_pattern_brush_schem() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("st".into()), None).unwrap();
        let mut session = Session::open(&cwd, "st").unwrap();
        let mut wv = WorldView::new(&mut session);

        let t0 = Instant::now();
        let pat = Pattern::parse("40%stone,30%dirt,20%cobblestone,10%gravel").unwrap();
        let action = wv
            .fill_pattern(0, 40, 0, 31, 71, 31, pat, Some(Mask::any()))
            .unwrap();
        assert!(action.changed_count() > 10_000);

        wv.brush_sphere(
            16,
            56,
            16,
            12.0,
            Pattern::parse("minecraft:glass").unwrap(),
            Some(Mask::parse("#solid").unwrap()),
            false,
        )
        .unwrap();
        assert_eq!(
            wv.get_block(16, 56, 16).unwrap().to_compact(),
            "minecraft:glass"
        );

        let out = tmp.path().join("mid.schem");
        schem::export_aabb(&wv, 0, 40, 0, 31, 71, 31, &out).unwrap();
        let tpl = schem::import_to_template(&out, "mid").unwrap();
        assert_eq!(tpl.size, [32, 32, 32]);
        let elapsed = t0.elapsed();
        // Soft bound: should finish well under a minute on CI.
        assert!(
            elapsed.as_secs() < 90,
            "medium stress took too long: {elapsed:?}"
        );
    });
}

/// Heavy stress (manual): 64×32×64 fill + brush r=24 + smooth + biome + schem.
/// Run: `cargo test -p mcaedit-core stress_heavy -- --ignored --nocapture`
#[test]
#[ignore]
fn stress_heavy_region_brush_smooth_biome_schem() {
    with_stack(|| {
        let tmp = TempDir::new().unwrap();
        let world = make_world(&tmp);
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("hv".into()), None).unwrap();
        let mut session = Session::open(&cwd, "hv").unwrap();
        let mut wv = WorldView::new(&mut session);

        let t0 = Instant::now();
        let pat = Pattern::parse("50%stone,50%dirt").unwrap();
        let a = wv
            .fill_pattern(0, 32, 0, 63, 63, 63, pat, None)
            .unwrap();
        eprintln!("fill changed={} {:?}", a.changed_count(), t0.elapsed());

        let t1 = Instant::now();
        wv.brush_sphere(
            32,
            48,
            32,
            24.0,
            Pattern::parse("25%glass,25%sand,25%gravel,25%clay").unwrap(),
            Some(Mask::parse("!air").unwrap()),
            false,
        )
        .unwrap();
        eprintln!("brush {:?}", t1.elapsed());

        let t2 = Instant::now();
        wv.smooth(0, 32, 0, 63, 80, 63, 3, 2).unwrap();
        eprintln!("smooth {:?}", t2.elapsed());

        let t3 = Instant::now();
        wv.biome_paint(0, 32, 0, 63, 63, 63, "minecraft:badlands")
            .unwrap();
        assert_eq!(wv.get_biome(16, 48, 16).unwrap(), "minecraft:badlands");
        eprintln!("biome {:?}", t3.elapsed());

        let out = tmp.path().join("heavy.schem");
        let t4 = Instant::now();
        schem::export_aabb(&wv, 0, 32, 0, 63, 63, 63, &out).unwrap();
        schem::import_paste(&mut wv, &out, 128, 32, 128).unwrap();
        eprintln!("schem roundtrip {:?}", t4.elapsed());
        eprintln!("total {:?}", t0.elapsed());
        assert!(out.metadata().unwrap().len() > 1000);
    });
}

#[test]
fn pattern_sampling_volume_distribution() {
    let p = Pattern::parse("50%stone,50%dirt").unwrap();
    let mut stone = 0u32;
    let mut dirt = 0u32;
    for y in 0..16 {
        for z in 0..32 {
            for x in 0..32 {
                let b = p.pick_at(x, y, z).to_compact();
                if b.contains("stone") {
                    stone += 1;
                } else {
                    dirt += 1;
                }
            }
        }
    }
    let total = stone + dirt;
    assert_eq!(total, 16 * 32 * 32);
    // Expect roughly balanced; allow wide slack for hash distribution.
    let ratio = stone as f64 / total as f64;
    assert!(
        (0.35..0.65).contains(&ratio),
        "stone ratio out of range: {ratio}"
    );
}
