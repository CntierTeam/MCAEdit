use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mcaedit_core::blockstate::BlockState;
use mcaedit_core::commit::commit_session;
use mcaedit_core::palette::SectionDiff;
use mcaedit_core::session::Session;
use mcaedit_core::template::Template;
use mcaedit_core::view::ViewScreenshotRequest;
use mcaedit_core::world::WorldView;
use serde_json::Value as JsonValue;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "mcaedit",
    version,
    about = "Offline MCA edit CLI for LLMs (session → edit → revert → commit)"
)]
struct Cli {
    /// Session id or path (default: latest under ./.mcaedit)
    #[arg(long, global = true, env = "MCAEDIT_SESSION")]
    session: Option<String>,

    /// Machine-readable output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Session {
        #[command(subcommand)]
        cmd: SessionCmd,
    },
    Inspect {
        #[command(subcommand)]
        cmd: InspectCmd,
    },
    Edit {
        #[command(subcommand)]
        cmd: EditCmd,
    },
    History {
        #[command(subcommand)]
        cmd: HistoryCmd,
    },
    Template {
        #[command(subcommand)]
        cmd: TemplateCmd,
    },
    View {
        #[command(subcommand)]
        cmd: ViewCmd,
    },
    /// Write working copy MCA files back to the source world
    Commit {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
enum TemplateCmd {
    /// Save AABB from current session as a reusable template
    Save {
        #[arg(long)]
        name: String,
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
    },
    List,
    Show {
        #[arg(long)]
        name: String,
    },
    /// Paste template into current session at origin
    Paste {
        #[arg(long)]
        name: String,
        #[arg(long, help = "x,y,z")]
        at: String,
    },
    Rm {
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCmd {
    Create {
        #[arg(long)]
        world: PathBuf,
        #[arg(long, default_value = "overworld")]
        dim: String,
        #[arg(long)]
        id: Option<String>,
        /// Collaboration label (agent / user)
        #[arg(long)]
        label: Option<String>,
    },
    /// List all sessions (multi-session collaboration)
    List,
    Status,
    /// Pull latest committed world into this session work copy
    Sync {
        #[arg(long)]
        force: bool,
    },
    Discard,
}

#[derive(Subcommand, Debug)]
enum InspectCmd {
    Summary {
        #[arg(long)]
        cx: Option<i32>,
        #[arg(long)]
        cz: Option<i32>,
    },
    Get {
        #[arg(long)]
        x: i32,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        z: i32,
    },
    Slice {
        #[arg(long)]
        y: i32,
        #[arg(long, help = "x,z")]
        from: String,
        #[arg(long, help = "x,z")]
        to: String,
    },
    /// AABB select → palette rebuild + ASCII 3D (ids)
    Select {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
    },
    Palette {
        #[arg(long)]
        cx: i32,
        #[arg(long)]
        cz: i32,
        #[arg(long)]
        sy: i8,
    },
    Entities {
        #[arg(long, help = "x,y,z")]
        near: Option<String>,
        #[arg(long, default_value_t = 32.0)]
        r: f64,
    },
}

#[derive(Subcommand, Debug)]
enum EditCmd {
    SetBlock {
        #[arg(long)]
        x: i32,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        z: i32,
        #[arg(long)]
        block: String,
    },
    Fill {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    /// Replace matching blocks in AABB (`--match air` = air-like)
    Replace {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long = "match")]
        match_block: String,
        #[arg(long = "with")]
        with_block: String,
    },
    Walls {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    Outline {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    Hollow {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
    },
    Overlay {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    Sphere {
        #[arg(long, help = "x,y,z center")]
        at: String,
        #[arg(long)]
        radius: f64,
        #[arg(long)]
        block: String,
        #[arg(long, default_value_t = false)]
        hollow: bool,
    },
    Cyl {
        #[arg(long, help = "x,z center")]
        at: String,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        radius: f64,
        #[arg(long)]
        height: i32,
        #[arg(long)]
        block: String,
        #[arg(long, default_value_t = false)]
        hollow: bool,
    },
    Stack {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long, default_value_t = 1)]
        n: i32,
        #[arg(long, default_value_t = 0)]
        dx: i32,
        #[arg(long, default_value_t = 0)]
        dy: i32,
        #[arg(long, default_value_t = 0)]
        dz: i32,
    },
    Move {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long, default_value_t = 0)]
        dx: i32,
        #[arg(long, default_value_t = 0)]
        dy: i32,
        #[arg(long, default_value_t = 0)]
        dz: i32,
    },
    Copy {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
    },
    Cut {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
    },
    Paste {
        #[arg(long, help = "x,y,z origin")]
        at: String,
    },
    /// Rotate session clipboard yaw (90/180/270)
    Rotate {
        #[arg(long)]
        yaw: i32,
    },
    /// Flip session clipboard on axis x|y|z
    Flip {
        #[arg(long)]
        axis: String,
    },
    /// Generate terrain into session work region/
    Gen {
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long, default_value = "overworld")]
        dim: String,
        #[arg(long, help = "chunk x,z")]
        from: String,
        #[arg(long, help = "chunk x,z")]
        to: String,
    },
    /// Recalculate sky/block light for chunk AABB
    FixLight {
        #[arg(long, help = "chunk x,z")]
        from: String,
        #[arg(long, help = "chunk x,z")]
        to: String,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long, default_value = "overworld")]
        dim: String,
    },
    /// Rebuild random-tick masks + step scheduled ticks (offline participate)
    TickParticipate {
        #[arg(long, help = "chunk x,z")]
        from: String,
        #[arg(long, help = "chunk x,z")]
        to: String,
        #[arg(long, default_value_t = 1)]
        rounds: u32,
        #[arg(long, default_value_t = 3)]
        speed: u32,
    },
    SetSection {
        #[arg(long)]
        cx: i32,
        #[arg(long)]
        cy: i32,
        #[arg(long)]
        cz: i32,
        /// JSON file path, or `-` for stdin
        #[arg(long)]
        file: String,
    },
    Entity {
        #[command(subcommand)]
        cmd: EntityCmd,
    },
}

#[derive(Subcommand, Debug)]
enum EntityCmd {
    Spawn {
        /// Entity NBT JSON file or `-` for stdin (must include id, Pos)
        #[arg(long)]
        file: String,
    },
    Rm {
        #[arg(long)]
        uuid: String,
        #[arg(long, default_value_t = 0)]
        x: i32,
        #[arg(long, default_value_t = 0)]
        z: i32,
    },
    Set {
        #[arg(long)]
        uuid: String,
        #[arg(long, default_value_t = 0)]
        x: i32,
        #[arg(long, default_value_t = 0)]
        z: i32,
        #[arg(long)]
        file: String,
    },
}

#[derive(Subcommand, Debug)]
enum HistoryCmd {
    List,
    Undo {
        #[arg(long, default_value_t = 1)]
        n: usize,
    },
    Redo {
        #[arg(long, default_value_t = 1)]
        n: usize,
    },
    Revert {
        #[arg(long)]
        to: usize,
    },
}

#[derive(Subcommand, Debug)]
enum ViewCmd {
    /// Offline 3D screenshot for LLM vision (PNG)
    Screenshot {
        #[arg(long, help = "x,y,z")]
        from: String,
        #[arg(long, help = "x,y,z")]
        to: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        #[arg(long, help = "x,y,z camera position")]
        camera: Option<String>,
        #[arg(long, help = "x,y,z look target")]
        look: Option<String>,
    },
}

fn main() -> Result<()> {
    // `mca` region encode/decode needs a large stack; Windows main-thread default (~1MiB)
    // overflows on first set-block. Mirror the integration-test worker stack.
    const STACK: usize = 16 * 1024 * 1024;
    std::thread::Builder::new()
        .name("mcaedit-main".into())
        .stack_size(STACK)
        .spawn(run)
        .context("spawn mcaedit worker")?
        .join()
        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    match cli.command {
        Commands::Session { cmd } => match cmd {
            SessionCmd::Create {
                world,
                dim,
                id,
                label,
            } => {
                let s = Session::create(&cwd, &world, &dim, id, label)?;
                print_session(&s, cli.json);
            }
            SessionCmd::List => {
                let list = Session::list(&cwd)?;
                if cli.json {
                    println!("{}", serde_json::to_string(&list)?);
                } else {
                    println!("sessions n={}", list.len());
                    for s in list {
                        let label = s.label.as_deref().unwrap_or("-");
                        println!(
                            "{} label={} dirty={} history={} world={}",
                            s.id,
                            label,
                            s.dirty,
                            s.history,
                            s.world.display()
                        );
                    }
                }
            }
            SessionCmd::Status => {
                let s = open_session(&cwd, cli.session.as_deref())?;
                print_session(&s, cli.json);
            }
            SessionCmd::Sync { force } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                for line in s.sync_from_source(force)? {
                    println!("{line}");
                }
                println!("synced=true dirty={}", s.meta.dirty);
            }
            SessionCmd::Discard => {
                let s = open_session(&cwd, cli.session.as_deref())?;
                let id = s.meta.id.clone();
                s.discard()?;
                println!("discarded={id}");
            }
        },
        Commands::Inspect { cmd } => {
            let mut s = open_session(&cwd, cli.session.as_deref())?;
            let world = WorldView::new(&mut s);
            match cmd {
                InspectCmd::Summary { cx, cz } => {
                    let cx = cx.unwrap_or(0);
                    let cz = cz.unwrap_or(0);
                    let line = world.summary_chunk(cx, cz)?;
                    println!("{line}");
                }
                InspectCmd::Get { x, y, z } => {
                    let b = world.get_block(x, y, z)?;
                    if cli.json {
                        println!("{}", serde_json::to_string(&b)?);
                    } else {
                        println!("block={} @ {x},{y},{z}", b.to_compact());
                    }
                }
                InspectCmd::Slice { y, from, to } => {
                    let (x1, z1) = parse_xz(&from)?;
                    let (x2, z2) = parse_xz(&to)?;
                    for line in world.slice(y, x1, z1, x2, z2)? {
                        println!("{line}");
                    }
                }
                InspectCmd::Select { from, to } => {
                    let (x1, y1, z1) = parse_xyz_i(&from)?;
                    let (x2, y2, z2) = parse_xyz_i(&to)?;
                    for line in world.select_box(x1, y1, z1, x2, y2, z2)? {
                        println!("{line}");
                    }
                }
                InspectCmd::Palette { cx, cz, sy } => {
                    let chunk = world.load_chunk(cx, cz)?;
                    let list = chunk.section_palette_list(sy)?;
                    if cli.json {
                        println!("{}", serde_json::to_string(&list)?);
                    } else {
                        println!("palette section={cx},{sy},{cz} n={}", list.len());
                        for (i, b) in list.iter().enumerate() {
                            println!("{i} {}", b.to_compact());
                        }
                    }
                }
                InspectCmd::Entities { near, r } => {
                    let _ = world
                        .session
                        .work_entities_dir();
                    // ensure copy for near area
                    let (x, y, z) = if let Some(near) = near {
                        parse_xyz(&near)?
                    } else {
                        (0.0, 0.0, 0.0)
                    };
                    // touch entity regions in radius
                    let min_cx = ((x - r).floor() as i32) >> 4;
                    let max_cx = ((x + r).floor() as i32) >> 4;
                    let min_cz = ((z - r).floor() as i32) >> 4;
                    let max_cz = ((z + r).floor() as i32) >> 4;
                    for cx in min_cx..=max_cx {
                        for cz in min_cz..=max_cz {
                            let (rx, rz) = mcaedit_core::region::region_coords(cx, cz);
                            let _ = mcaedit_core::region::copy_region_if_needed(
                                &world.session.source_entities_dir(),
                                &world.session.work_entities_dir(),
                                rx,
                                rz,
                            )?;
                        }
                    }
                    let ents = world.entity_store().list_near(x, y, z, r)?;
                    println!("entities n={}", ents.len());
                    for e in ents.iter().take(50) {
                        println!("{}", e.brief());
                    }
                }
            }
        }
        Commands::Edit { cmd } => {
            let mut s = open_session(&cwd, cli.session.as_deref())?;
            let mut world = WorldView::new(&mut s);
            match cmd {
                EditCmd::Copy { from, to } => {
                    let (x1, y1, z1) = parse_xyz_i(&from)?;
                    let (x2, y2, z2) = parse_xyz_i(&to)?;
                    let tpl = world.clipboard_copy(x1, y1, z1, x2, y2, z2)?;
                    if cli.json {
                        println!("{}", serde_json::to_string(&tpl)?);
                    } else {
                        for line in tpl.brief_lines() {
                            println!("{line}");
                        }
                        println!("clipboard=saved");
                    }
                }
                EditCmd::Rotate { yaw } => {
                    let tpl = world.clipboard_rotate_yaw(yaw)?;
                    if cli.json {
                        println!("{}", serde_json::to_string(&tpl)?);
                    } else {
                        for line in tpl.brief_lines() {
                            println!("{line}");
                        }
                        println!("clipboard=rotated yaw={yaw}");
                    }
                }
                EditCmd::Flip { axis } => {
                    let ch = axis
                        .chars()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("axis required"))?;
                    let tpl = world.clipboard_flip(ch)?;
                    if cli.json {
                        println!("{}", serde_json::to_string(&tpl)?);
                    } else {
                        for line in tpl.brief_lines() {
                            println!("{line}");
                        }
                        println!("clipboard=flipped axis={axis}");
                    }
                }
                EditCmd::Gen {
                    seed,
                    dim,
                    from,
                    to,
                } => {
                    let (cx1, cz1) = parse_xz(&from)?;
                    let (cx2, cz2) = parse_xz(&to)?;
                    let lines = world.gen_terrain(seed, &dim, cx1, cz1, cx2, cz2)?;
                    for line in lines {
                        println!("{line}");
                    }
                }
                EditCmd::FixLight {
                    from,
                    to,
                    seed,
                    dim,
                } => {
                    let (cx1, cz1) = parse_xz(&from)?;
                    let (cx2, cz2) = parse_xz(&to)?;
                    let lines = world.fix_light(cx1, cz1, cx2, cz2, seed, &dim)?;
                    for line in lines {
                        println!("{line}");
                    }
                }
                EditCmd::TickParticipate {
                    from,
                    to,
                    rounds,
                    speed,
                } => {
                    let (cx1, cz1) = parse_xz(&from)?;
                    let (cx2, cz2) = parse_xz(&to)?;
                    let lines = world.tick_participate(cx1, cz1, cx2, cz2, rounds, speed)?;
                    for line in lines {
                        println!("{line}");
                    }
                }
                other => {
                    let action = match other {
                        EditCmd::SetBlock { x, y, z, block } => {
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.set_block(x, y, z, block)?
                        }
                        EditCmd::Fill { from, to, block } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.fill(x1, y1, z1, x2, y2, z2, block)?
                        }
                        EditCmd::Replace {
                            from,
                            to,
                            match_block,
                            with_block,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let match_block = mcaedit_core::ops::parse_match_filter(&match_block)
                                .map_err(|e| anyhow::anyhow!(e))?;
                            let with_block = BlockState::parse(&with_block)
                                .map_err(|e| anyhow::anyhow!(e))?;
                            world.replace(x1, y1, z1, x2, y2, z2, match_block, with_block)?
                        }
                        EditCmd::Walls { from, to, block } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.walls(x1, y1, z1, x2, y2, z2, block)?
                        }
                        EditCmd::Outline { from, to, block } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.outline(x1, y1, z1, x2, y2, z2, block)?
                        }
                        EditCmd::Hollow { from, to } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.hollow(x1, y1, z1, x2, y2, z2)?
                        }
                        EditCmd::Overlay { from, to, block } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.overlay(x1, y1, z1, x2, y2, z2, block)?
                        }
                        EditCmd::Sphere {
                            at,
                            radius,
                            block,
                            hollow,
                        } => {
                            let (cx, cy, cz) = parse_xyz_i(&at)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.sphere(cx, cy, cz, radius, block, hollow)?
                        }
                        EditCmd::Cyl {
                            at,
                            y,
                            radius,
                            height,
                            block,
                            hollow,
                        } => {
                            let (cx, cz) = parse_xz(&at)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.cyl(cx, cz, y, radius, height, block, hollow)?
                        }
                        EditCmd::Stack {
                            from,
                            to,
                            n,
                            dx,
                            dy,
                            dz,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.stack(x1, y1, z1, x2, y2, z2, n, dx, dy, dz)?
                        }
                        EditCmd::Move {
                            from,
                            to,
                            dx,
                            dy,
                            dz,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.move_region(x1, y1, z1, x2, y2, z2, dx, dy, dz)?
                        }
                        EditCmd::Cut { from, to } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.clipboard_cut(x1, y1, z1, x2, y2, z2)?
                        }
                        EditCmd::Paste { at } => {
                            let (x, y, z) = parse_xyz_i(&at)?;
                            world.clipboard_paste(x, y, z)?
                        }
                        EditCmd::SetSection {
                            cx,
                            cy,
                            cz,
                            file,
                        } => {
                            let text = read_file_or_stdin(&file)?;
                            let after: SectionDiff = serde_json::from_str(&text)
                                .or_else(|_| {
                                    let v: JsonValue = serde_json::from_str(&text)?;
                                    parse_section_diff_loose(&v)
                                })
                                .context("parse section diff")?;
                            world.set_section(cx, cy, cz, after)?
                        }
                        EditCmd::Entity { cmd } => match cmd {
                            EntityCmd::Spawn { file } => {
                                let text = read_file_or_stdin(&file)?;
                                let nbt: JsonValue = serde_json::from_str(&text)?;
                                world.entity_spawn(nbt)?
                            }
                            EntityCmd::Rm { uuid, x, z } => {
                                world.entity_remove(&uuid, x, z)?
                            }
                            EntityCmd::Set {
                                uuid,
                                x,
                                z,
                                file,
                            } => {
                                let text = read_file_or_stdin(&file)?;
                                let nbt: JsonValue = serde_json::from_str(&text)?;
                                world.entity_set(&uuid, x, z, nbt)?
                            }
                        },
                        EditCmd::Copy { .. }
                        | EditCmd::Rotate { .. }
                        | EditCmd::Flip { .. }
                        | EditCmd::Gen { .. }
                        | EditCmd::FixLight { .. }
                        | EditCmd::TickParticipate { .. } => unreachable!(),
                    };
                    if cli.json {
                        println!("{}", serde_json::to_string(&action)?);
                    } else {
                        println!(
                            "action={} changed={} dirty=true desc={}",
                            action.id,
                            action.changed_count(),
                            action.description
                        );
                    }
                }
            }
        }
        Commands::History { cmd } => {
            let mut s = open_session(&cwd, cli.session.as_deref())?;
            match cmd {
                HistoryCmd::List => {
                    for a in s.history.list()? {
                        let mark = if (a.id as usize) <= s.history.position() {
                            "*"
                        } else {
                            " "
                        };
                        println!("{mark} #{:<4} {}", a.id, a.description);
                    }
                    println!("cursor={}/{}", s.history.position(), s.history.size());
                }
                HistoryCmd::Undo { n } => {
                    let mut world = WorldView::new(&mut s);
                    let done = world.undo(n)?;
                    println!("undone={}", done.len());
                    for a in done {
                        println!("#{} {}", a.id, a.description);
                    }
                }
                HistoryCmd::Redo { n } => {
                    let mut world = WorldView::new(&mut s);
                    let done = world.redo(n)?;
                    println!("redone={}", done.len());
                    for a in done {
                        println!("#{} {}", a.id, a.description);
                    }
                }
                HistoryCmd::Revert { to } => {
                    let mut world = WorldView::new(&mut s);
                    let done = world.revert_to(to)?;
                    println!("reverted_to={to} steps={}", done.len());
                }
            }
        }
        Commands::Template { cmd } => match cmd {
            TemplateCmd::Save { name, from, to } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let world = WorldView::new(&mut s);
                let (x1, y1, z1) = parse_xyz_i(&from)?;
                let (x2, y2, z2) = parse_xyz_i(&to)?;
                let tpl = Template::capture(&world, &name, x1, y1, z1, x2, y2, z2)?;
                let path = tpl.save_to_disk(&cwd)?;
                for line in tpl.brief_lines() {
                    println!("{line}");
                }
                println!("saved={}", path.display());
            }
            TemplateCmd::List => {
                let names = Template::list(&cwd)?;
                println!("templates n={}", names.len());
                for n in names {
                    println!("{n}");
                }
            }
            TemplateCmd::Show { name } => {
                let tpl = Template::load(&cwd, &name)?;
                if cli.json {
                    println!("{}", serde_json::to_string(&tpl)?);
                } else {
                    for line in tpl.brief_lines() {
                        println!("{line}");
                    }
                    println!("palette n={}", tpl.palette.len());
                    for (i, b) in tpl.palette.iter().enumerate() {
                        println!("{i} {b}");
                    }
                }
            }
            TemplateCmd::Paste { name, at } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let mut world = WorldView::new(&mut s);
                let (x, y, z) = parse_xyz_i(&at)?;
                let tpl = Template::load(&cwd, &name)?;
                let action = tpl.paste_into(&mut world, x, y, z)?;
                println!(
                    "action={} changed={} dirty=true desc={}",
                    action.id,
                    action.changed_count(),
                    action.description
                );
            }
            TemplateCmd::Rm { name } => {
                Template::delete(&cwd, &name)?;
                println!("deleted={name}");
            }
        },
        Commands::View { cmd } => {
            let mut s = open_session(&cwd, cli.session.as_deref())?;
            let world = WorldView::new(&mut s);
            match cmd {
                ViewCmd::Screenshot {
                    from,
                    to,
                    out,
                    width,
                    height,
                    camera,
                    look,
                } => {
                    let (x1, y1, z1) = parse_xyz_i(&from)?;
                    let (x2, y2, z2) = parse_xyz_i(&to)?;
                    let camera = camera.as_deref().map(parse_xyz_f32).transpose()?;
                    let look = look.as_deref().map(parse_xyz_f32).transpose()?;
                    let out_path = out.unwrap_or_else(|| {
                        let ts = unix_ts();
                        world.session.root.join(format!("view-{ts}.png"))
                    });
                    let req = ViewScreenshotRequest {
                        from: (x1, y1, z1),
                        to: (x2, y2, z2),
                        width,
                        height,
                        camera,
                        look,
                        out: out_path,
                    };
                    let rep = world.view_screenshot(&req)?;
                    println!(
                        "screenshot={} size={}x{} blocks={} faces={} triangles={}",
                        rep.out.display(),
                        rep.width,
                        rep.height,
                        rep.blocks,
                        rep.faces,
                        rep.triangles
                    );
                }
            }
        }
        Commands::Commit { dry_run } => {
            let mut s = open_session(&cwd, cli.session.as_deref())?;
            let report = commit_session(&cwd, &mut s, dry_run)?;
            for line in report {
                println!("{line}");
            }
            if dry_run {
                println!("dry_run=true");
            } else {
                println!("committed=true dirty=false");
            }
        }
    }
    Ok(())
}

fn open_session(cwd: &std::path::Path, session: Option<&str>) -> Result<Session> {
    match session {
        Some(id) => Ok(Session::open(cwd, id)?),
        None => Ok(Session::open_default(cwd)?),
    }
}

fn print_session(s: &Session, json: bool) {
    if json {
        println!("{}", serde_json::to_string(&s.meta).unwrap());
    } else {
        for line in s.status_lines() {
            println!("{line}");
        }
    }
}

fn parse_xz(s: &str) -> Result<(i32, i32)> {
    let parts: Vec<_> = s.split(',').collect();
    if parts.len() != 2 {
        bail!("expected x,z got `{s}`");
    }
    Ok((parts[0].parse()?, parts[1].parse()?))
}

fn parse_xyz(s: &str) -> Result<(f64, f64, f64)> {
    let parts: Vec<_> = s.split(',').collect();
    if parts.len() != 3 {
        bail!("expected x,y,z got `{s}`");
    }
    Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
}

fn parse_xyz_i(s: &str) -> Result<(i32, i32, i32)> {
    let parts: Vec<_> = s.split(',').collect();
    if parts.len() != 3 {
        bail!("expected x,y,z got `{s}`");
    }
    Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
}

fn parse_xyz_f32(s: &str) -> Result<(f32, f32, f32)> {
    let (x, y, z) = parse_xyz(s)?;
    Ok((x as f32, y as f32, z as f32))
}

fn unix_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_file_or_stdin(path: &str) -> Result<String> {
    if path == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        Ok(std::fs::read_to_string(path)?)
    }
}

fn parse_section_diff_loose(v: &JsonValue) -> Result<SectionDiff> {
    if let Ok(d) = serde_json::from_value::<SectionDiff>(v.clone()) {
        return Ok(d);
    }
    let cells = v
        .get("cells")
        .and_then(|c| c.as_object())
        .ok_or_else(|| anyhow::anyhow!("need cells object"))?;
    let mut diff = SectionDiff::default();
    for (k, val) in cells {
        let state = match val {
            JsonValue::String(s) => BlockState::parse(s).map_err(|e| anyhow::anyhow!(e))?,
            other => serde_json::from_value(other.clone())?,
        };
        let mut parts = k.split(',');
        let x: u8 = parts.next().unwrap_or("0").parse()?;
        let y: u8 = parts.next().unwrap_or("0").parse()?;
        let z: u8 = parts.next().unwrap_or("0").parse()?;
        diff.set(x, y, z, state);
    }
    Ok(diff)
}
