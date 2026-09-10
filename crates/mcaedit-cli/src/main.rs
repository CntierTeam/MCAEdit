use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mcaedit_core::blockstate::BlockState;
use mcaedit_core::commit::commit_session;
use mcaedit_core::palette::SectionDiff;
use mcaedit_core::session::Session;
use mcaedit_core::template::Template;
use mcaedit_core::view::ViewScreenshotRequest;
#[cfg(feature = "preview")]
use mcaedit_core::view::suggest_preview_aabb;
use mcaedit_core::world::WorldView;
use serde_json::Value as JsonValue;
use std::path::PathBuf;

#[cfg(feature = "preview")]
mod preview;

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
    /// Sponge schematic (.schem) import / export
    Schem {
        #[command(subcommand)]
        cmd: SchemCmd,
    },
    /// Vanilla structure template (.nbt) import / export / place
    Structure {
        #[command(subcommand)]
        cmd: StructureCmd,
    },
    /// Create world skeleton / manage level.dat
    World {
        #[command(subcommand)]
        cmd: WorldCmd,
    },
    /// Read / patch level.dat
    Level {
        #[command(subcommand)]
        cmd: LevelCmd,
    },
    View {
        #[command(subcommand)]
        cmd: ViewCmd,
    },
    /// Live 3D preview window (alias of `view preview`)
    Preview {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z focus AABB min (default: auto from work region)")]
        from: Option<String>,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z focus AABB max")]
        to: Option<String>,
        /// Poll interval for session work-region reload (milliseconds)
        #[arg(long, default_value_t = 400)]
        watch: u64,
        #[arg(long, default_value_t = 960)]
        width: u32,
        #[arg(long, default_value_t = 540)]
        height: u32,
        /// Minecraft client jar or versions/<ver> directory (default: auto-detect 26.2)
        #[arg(long, env = "MCAEDIT_MINECRAFT_JAR")]
        minecraft: Option<PathBuf>,
        /// Explicit assets/client jar (alias of --minecraft; env MCAEDIT_ASSETS_JAR)
        #[arg(long = "assets-jar", env = "MCAEDIT_ASSETS_JAR")]
        assets_jar: Option<PathBuf>,
        /// Force solid palette colors (skip jar textures)
        #[arg(long)]
        no_textures: bool,
        /// Max AABB cells for mesh (default 2e6 / env MCAEDIT_VIEW_MAX_CELLS)
        #[arg(long = "max-cells", env = "MCAEDIT_VIEW_MAX_CELLS")]
        max_cells: Option<usize>,
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        at: String,
    },
    Rm {
        #[arg(long)]
        name: String,
    },
    /// Export template to Sponge .schem
    ExportSchem {
        #[arg(long)]
        name: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Import .schem as a named template
    ImportSchem {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum SchemCmd {
    /// Export AABB to .schem (Sponge v2)
    Export {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Import/paste .schem at origin
    Import {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        at: String,
    },
    /// Show .schem metadata
    Info {
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum StructureCmd {
    /// List structure .nbt under world generated/ / datapacks/ / structures/
    List {
        #[arg(long)]
        world: Option<PathBuf>,
    },
    /// Show structure .nbt metadata
    Info {
        #[arg(long)]
        file: PathBuf,
    },
    /// Place / import structure .nbt into the session at origin
    Place {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        at: String,
        /// Rotate yaw degrees: 0/90/180/270
        #[arg(long, default_value_t = 0)]
        rotation: i32,
        /// Mirror axis: x or z
        #[arg(long)]
        mirror: Option<String>,
        /// Skip entities in the structure
        #[arg(long)]
        no_entities: bool,
    },
    /// Export AABB to vanilla structure .nbt
    Export {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long = "data-version")]
        data_version: Option<i32>,
    },
    /// Import .nbt as a named template (dense)
    Import {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// Clear chunk structure starts/references overlapping AABB
    ClearRefs {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long = "no-starts")]
        no_starts: bool,
        #[arg(long = "no-references")]
        no_references: bool,
    },
}

#[derive(Subcommand, Debug)]
enum WorldCmd {
    /// Create empty world dirs + level.dat (default MC 26.2)
    Create {
        #[arg(long)]
        path: PathBuf,
        #[arg(long = "name", default_value = "world")]
        level_name: String,
        #[arg(long, default_value_t = 0)]
        seed: i64,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z", default_value = "0,64,0")]
        spawn: String,
        /// 0=survival 1=creative 2=adventure 3=spectator
        #[arg(long = "game-type", default_value_t = 1)]
        game_type: i32,
        /// noise | flat
        #[arg(long, default_value = "noise")]
        generator: String,
        /// e.g. 26.2, 1.21.4, 1.20.1, 1.18.2
        #[arg(long = "mc", default_value = "26.2")]
        mc: String,
        #[arg(long = "data-version")]
        data_version: Option<i32>,
        /// anvil | linear
        #[arg(long = "region-format", default_value = "anvil")]
        region_format: String,
        #[arg(long)]
        all_dims: bool,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
enum LevelCmd {
    /// Show level.dat summary
    Info {
        #[arg(long)]
        world: PathBuf,
    },
    /// Create or overwrite level.dat (and modern sidecars when applicable)
    Write {
        #[arg(long)]
        world: PathBuf,
        #[arg(long = "name")]
        level_name: Option<String>,
        #[arg(long)]
        seed: Option<i64>,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        spawn: Option<String>,
        #[arg(long = "game-type")]
        game_type: Option<i32>,
        #[arg(long = "mc")]
        mc: Option<String>,
        #[arg(long = "data-version")]
        data_version: Option<i32>,
        #[arg(long, default_value = "noise")]
        generator: String,
        #[arg(long)]
        force: bool,
    },
    /// Patch fields on existing level.dat
    Patch {
        #[arg(long)]
        world: PathBuf,
        #[arg(long = "name")]
        level_name: Option<String>,
        #[arg(long)]
        seed: Option<i64>,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        spawn: Option<String>,
        #[arg(long = "game-type")]
        game_type: Option<i32>,
        #[arg(long = "data-version")]
        data_version: Option<i32>,
        #[arg(long = "version-name")]
        version_name: Option<String>,
        #[arg(long)]
        touch: bool,
    },
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)] // clap Create carries many PathBuf/String flags
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
        /// Create level.dat + region dirs if world is empty / missing
        #[arg(long)]
        bootstrap: bool,
        #[arg(long = "name")]
        level_name: Option<String>,
        #[arg(long)]
        seed: Option<i64>,
        #[arg(long = "mc", default_value = "26.2")]
        mc: String,
        #[arg(long = "data-version")]
        data_version: Option<i32>,
        #[arg(long, default_value = "noise")]
        generator: String,
        #[arg(long = "region-format", default_value = "anvil")]
        region_format: String,
        #[arg(long)]
        force: bool,
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
    /// Claim soft AABB lease for multi-session coordination
    Lease {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
    },
    /// Clear this session lease
    #[command(name = "lease-clear")]
    LeaseClear,
    /// List all soft leases
    #[command(name = "lease-list")]
    LeaseList,
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
        #[arg(long, allow_hyphen_values = true, help = "x,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,z")]
        to: String,
    },
    /// AABB select → palette rebuild + ASCII 3D (ids)
    Select {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
    },
    /// Large AABB block counts (no ASCII); for 验收
    #[command(name = "summary-box")]
    SummaryBox {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        near: Option<String>,
        #[arg(long, default_value_t = 32.0)]
        r: f64,
    },
    /// Chunk structure starts / References summary
    Structures {
        #[arg(long)]
        cx: i32,
        #[arg(long)]
        cz: i32,
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        /// Block or % pattern (e.g. minecraft:stone or 50%stone,50%dirt)
        #[arg(long)]
        block: Option<String>,
        #[arg(long, help = "alias of --block; supports % pattern")]
        pattern: Option<String>,
        /// Only touch matching blocks (air|stone|#solid|a&b|!air)
        #[arg(long)]
        mask: Option<String>,
        #[arg(long = "mask-exclude", help = "exclude mask (OR list)")]
        mask_exclude: Option<String>,
    },
    /// Replace matching blocks in AABB (`--match air` = air-like; `--with` may be % pattern)
    Replace {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long = "match", help = "mask / block filter")]
        match_block: Option<String>,
        #[arg(long, help = "alias of --match (compose mask)")]
        mask: Option<String>,
        #[arg(long = "mask-exclude")]
        mask_exclude: Option<String>,
        #[arg(long = "with", help = "block or % pattern")]
        with_block: Option<String>,
        #[arg(long, help = "alias of --with")]
        pattern: Option<String>,
    },
    Walls {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    Outline {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    Hollow {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
    },
    Overlay {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
    },
    Sphere {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z center")]
        at: String,
        #[arg(long)]
        radius: f64,
        #[arg(long)]
        block: String,
        #[arg(long, default_value_t = false)]
        hollow: bool,
    },
    Cyl {
        #[arg(long, allow_hyphen_values = true, help = "x,z center")]
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long, default_value_t = 0)]
        dx: i32,
        #[arg(long, default_value_t = 0)]
        dy: i32,
        #[arg(long, default_value_t = 0)]
        dz: i32,
    },
    Copy {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
    },
    Cut {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
    },
    Paste {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z origin")]
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
    /// Brush apply at a point (sphere / cyl / clipboard / biome)
    Brush {
        #[command(subcommand)]
        cmd: BrushCmd,
    },
    /// Heightmap / surface smooth over AABB
    Smooth {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long, default_value_t = 1)]
        iterations: u32,
        /// Neighbor kernel radius (Chebyshev), default 1
        #[arg(long, default_value_t = 1)]
        kernel: i32,
    },
    /// 3D voxel neighbourhood smooth (majority vote) over AABB
    Smooth3d {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long, default_value_t = 1)]
        iterations: u32,
        /// Neighbor kernel radius (Chebyshev), default 1
        #[arg(long, default_value_t = 1)]
        kernel: i32,
        /// Vote air vs solid first; solid winners use majority solid neighbour
        #[arg(long, default_value_t = false)]
        solid: bool,
    },
    /// Paint biomes in AABB (4×4×4 resolution per section)
    Biome {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long, help = "minecraft:plains or plains")]
        biome: String,
    },
    /// Generate terrain into session work region/
    Gen {
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long, default_value = "overworld")]
        dim: String,
        #[arg(long, allow_hyphen_values = true, help = "chunk x,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "chunk x,z")]
        to: String,
    },
    /// Recalculate sky/block light only (preserves blocks/palettes/entities)
    FixLight {
        #[arg(long, allow_hyphen_values = true, help = "chunk x,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "chunk x,z")]
        to: String,
        /// Dimension extents for the lighting proto only — does not regenerate terrain
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long, default_value = "overworld")]
        dim: String,
    },
    /// Offline tick: scheduled queues + approximate random-tick growth
    #[command(name = "tick", alias = "tick-participate")]
    Tick {
        #[arg(long, allow_hyphen_values = true, help = "chunk x,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "chunk x,z")]
        to: String,
        #[arg(long, default_value_t = 1)]
        rounds: u32,
        #[arg(long, default_value_t = 3)]
        speed: u32,
    },
    /// Pillar grid / 柱网 inside AABB
    #[command(name = "grid", alias = "colonnade")]
    Grid {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long = "spacing-x", default_value_t = 4)]
        spacing_x: i32,
        #[arg(long = "spacing-z", default_value_t = 4)]
        spacing_z: i32,
        #[arg(long)]
        block: Option<String>,
        #[arg(long)]
        pattern: Option<String>,
    },
    /// Roof tile rows / 瓦垄
    #[command(name = "roof-rows")]
    RoofRows {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long, default_value = "z", help = "row axis x|z")]
        axis: String,
        #[arg(long, default_value_t = 2)]
        period: i32,
        #[arg(long)]
        block: String,
        #[arg(long, help = "optional stairs block for odd rows")]
        stairs: Option<String>,
        #[arg(long = "stairs-facing", default_value = "north")]
        stairs_facing: String,
        #[arg(long = "stairs-half", default_value = "bottom")]
        stairs_half: String,
    },
    /// Fill AABB with oriented stairs
    Stairs {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        block: String,
        #[arg(long, default_value = "north")]
        facing: String,
        #[arg(long, default_value = "bottom")]
        half: String,
        #[arg(long, default_value = "straight")]
        shape: String,
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
enum BrushCmd {
    /// Sphere brush at center
    Sphere {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z center")]
        at: String,
        #[arg(long)]
        radius: f64,
        #[arg(long)]
        block: Option<String>,
        #[arg(long)]
        pattern: Option<String>,
        #[arg(long)]
        mask: Option<String>,
        #[arg(long = "mask-exclude")]
        mask_exclude: Option<String>,
        #[arg(long, default_value_t = false)]
        hollow: bool,
    },
    /// Vertical cylinder brush
    Cyl {
        #[arg(long, allow_hyphen_values = true, help = "x,z center")]
        at: String,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        radius: f64,
        #[arg(long)]
        height: i32,
        #[arg(long)]
        block: Option<String>,
        #[arg(long)]
        pattern: Option<String>,
        #[arg(long)]
        mask: Option<String>,
        #[arg(long = "mask-exclude")]
        mask_exclude: Option<String>,
        #[arg(long, default_value_t = false)]
        hollow: bool,
    },
    /// Paste clipboard at point; optional sphere clip + mask
    Clipboard {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z origin")]
        at: String,
        #[arg(long)]
        radius: Option<f64>,
        #[arg(long)]
        mask: Option<String>,
        #[arg(long = "mask-exclude")]
        mask_exclude: Option<String>,
    },
    /// Biome sphere brush (world-space shape, 4×4×4 cells)
    Biome {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z center")]
        at: String,
        #[arg(long)]
        radius: f64,
        #[arg(long, help = "minecraft:desert or desert")]
        biome: String,
        #[arg(long)]
        mask: Option<String>,
        #[arg(long = "mask-exclude")]
        mask_exclude: Option<String>,
        #[arg(long, default_value_t = false)]
        hollow: bool,
    },
    /// Biome vertical cylinder brush
    #[command(name = "biome-cyl")]
    BiomeCyl {
        #[arg(long, allow_hyphen_values = true, help = "x,z center")]
        at: String,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        radius: f64,
        #[arg(long)]
        height: i32,
        #[arg(long, help = "minecraft:desert or desert")]
        biome: String,
        #[arg(long)]
        mask: Option<String>,
        #[arg(long = "mask-exclude")]
        mask_exclude: Option<String>,
        #[arg(long, default_value_t = false)]
        hollow: bool,
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
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        from: String,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z")]
        to: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z camera position")]
        camera: Option<String>,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z look target")]
        look: Option<String>,
        /// Minecraft client jar or versions/<ver> directory (default: auto-detect 26.2)
        #[arg(long, env = "MCAEDIT_MINECRAFT_JAR")]
        minecraft: Option<PathBuf>,
        /// Explicit assets/client jar (env MCAEDIT_ASSETS_JAR)
        #[arg(long = "assets-jar", env = "MCAEDIT_ASSETS_JAR")]
        assets_jar: Option<PathBuf>,
        /// Force solid palette colors (skip jar textures)
        #[arg(long)]
        no_textures: bool,
        /// Max AABB cells for mesh (default 2e6 / env MCAEDIT_VIEW_MAX_CELLS)
        #[arg(long = "max-cells", env = "MCAEDIT_VIEW_MAX_CELLS")]
        max_cells: Option<usize>,
    },
    /// Live native preview window (watch session work copy)
    Preview {
        #[arg(long, allow_hyphen_values = true, help = "x,y,z focus AABB min (default: auto from work region)")]
        from: Option<String>,
        #[arg(long, allow_hyphen_values = true, help = "x,y,z focus AABB max")]
        to: Option<String>,
        /// Poll interval for session work-region reload (milliseconds)
        #[arg(long, default_value_t = 400)]
        watch: u64,
        #[arg(long, default_value_t = 960)]
        width: u32,
        #[arg(long, default_value_t = 540)]
        height: u32,
        /// Minecraft client jar or versions/<ver> directory (default: auto-detect 26.2)
        #[arg(long, env = "MCAEDIT_MINECRAFT_JAR")]
        minecraft: Option<PathBuf>,
        /// Explicit assets/client jar (env MCAEDIT_ASSETS_JAR)
        #[arg(long = "assets-jar", env = "MCAEDIT_ASSETS_JAR")]
        assets_jar: Option<PathBuf>,
        /// Force solid palette colors (skip jar textures)
        #[arg(long)]
        no_textures: bool,
        /// Max AABB cells for mesh (default 2e6 / env MCAEDIT_VIEW_MAX_CELLS)
        #[arg(long = "max-cells", env = "MCAEDIT_VIEW_MAX_CELLS")]
        max_cells: Option<usize>,
    },
}

fn main() -> Result<()> {
    // Preview owns a winit event loop — must stay on the OS main thread.
    // Other commands spawn a 16MiB stack worker (Windows MCA encode overflows ~1MiB).
    if is_preview_invocation() {
        return run();
    }
    const STACK: usize = 16 * 1024 * 1024;
    std::thread::Builder::new()
        .name("mcaedit-main".into())
        .stack_size(STACK)
        .spawn(run)
        .context("spawn mcaedit worker")?
        .join()
        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
}

/// True for `mcaedit preview` / `mcaedit view preview` (global flags ignored).
fn is_preview_invocation() -> bool {
    let mut args = std::env::args().skip(1);
    let mut positionals = Vec::new();
    while let Some(a) = args.next() {
        if a == "--session" {
            let _ = args.next();
            continue;
        }
        if a.starts_with("--session=") || a == "--json" || a == "-h" || a == "--help" || a == "-V" || a == "--version" {
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        positionals.push(a);
    }
    matches!(
        positionals
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        ["preview", ..] | ["view", "preview", ..]
    )
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
                bootstrap,
                level_name,
                seed,
                mc,
                data_version,
                generator,
                region_format,
                force,
            } => {
                let version = mcaedit_core::mc_version::ResolvedVersion::resolve(
                    Some(&mc),
                    data_version,
                )?;
                let opts = mcaedit_core::CreateSessionOpts {
                    bootstrap,
                    force_level: force,
                    level_name,
                    seed,
                    version: Some(version),
                    generator: Some(mcaedit_core::level::GeneratorKind::parse(&generator)?),
                    region_format: Some(mcaedit_core::level::RegionFormat::parse(
                        &region_format,
                    )?),
                    data_version,
                };
                let s = Session::create_with_opts(&cwd, &world, &dim, id, label, opts)?;
                if let Some(note) = &s.bootstrap_note {
                    println!("{note}");
                }
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
            SessionCmd::Lease { from, to } => {
                let s = open_session(&cwd, cli.session.as_deref())?;
                let (x1, y1, z1) = parse_xyz_i(&from)?;
                let (x2, y2, z2) = parse_xyz_i(&to)?;
                let (lease, warnings) =
                    Session::set_lease(&cwd, &s, [x1, y1, z1], [x2, y2, z2])?;
                for w in warnings {
                    println!("warn={w}");
                }
                println!(
                    "lease=ok session={} from={},{},{} to={},{},{}",
                    lease.session,
                    lease.from[0],
                    lease.from[1],
                    lease.from[2],
                    lease.to[0],
                    lease.to[1],
                    lease.to[2]
                );
            }
            SessionCmd::LeaseClear => {
                let s = open_session(&cwd, cli.session.as_deref())?;
                let cleared = Session::clear_lease(&cwd, &s.meta.id)?;
                println!("lease_cleared={cleared}");
            }
            SessionCmd::LeaseList => {
                let list = Session::list_leases(&cwd)?;
                println!("leases n={}", list.len());
                for lease in list {
                    println!(
                        "session={} label={} from={},{},{} to={},{},{} updated={}",
                        lease.session,
                        lease.label.as_deref().unwrap_or("-"),
                        lease.from[0],
                        lease.from[1],
                        lease.from[2],
                        lease.to[0],
                        lease.to[1],
                        lease.to[2],
                        lease.updated_at
                    );
                }
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
                InspectCmd::SummaryBox { from, to } => {
                    let (x1, y1, z1) = parse_xyz_i(&from)?;
                    let (x2, y2, z2) = parse_xyz_i(&to)?;
                    for line in world.summary_box(x1, y1, z1, x2, y2, z2)? {
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
                InspectCmd::Structures { cx, cz } => {
                    let chunk = world.load_chunk(cx, cz)?;
                    println!("chunk={cx},{cz} DataVersion={:?}", chunk.data_version());
                    for line in mcaedit_core::structure::chunk_structure_summary(&chunk) {
                        println!("{line}");
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
                EditCmd::Tick {
                    from,
                    to,
                    rounds,
                    speed,
                } => {
                    let (cx1, cz1) = parse_xz(&from)?;
                    let (cx2, cz2) = parse_xz(&to)?;
                    let (lines, action) =
                        world.tick_offline(cx1, cz1, cx2, cz2, rounds, speed)?;
                    for line in lines {
                        println!("{line}");
                    }
                    if let Some(action) = action {
                        if cli.json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "id": action.id,
                                    "description": action.description,
                                    "changed": action.changed_count(),
                                })
                            );
                        } else {
                            println!(
                                "action={} changed={} ({})",
                                action.id,
                                action.changed_count(),
                                action.description
                            );
                        }
                    }
                }
                other => {
                    let action = match other {
                        EditCmd::SetBlock { x, y, z, block } => {
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            world.set_block(x, y, z, block)?
                        }
                        EditCmd::Fill {
                            from,
                            to,
                            block,
                            pattern,
                            mask,
                            mask_exclude,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let pattern = parse_pattern_arg(pattern.as_deref(), block.as_deref())?;
                            let mask = parse_mask_opt(mask.as_deref(), mask_exclude.as_deref())?;
                            world.fill_pattern(x1, y1, z1, x2, y2, z2, pattern, mask)?
                        }
                        EditCmd::Replace {
                            from,
                            to,
                            match_block,
                            mask,
                            mask_exclude,
                            with_block,
                            pattern,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let mask_src = mask
                                .as_deref()
                                .or(match_block.as_deref())
                                .ok_or_else(|| anyhow::anyhow!("need --match or --mask"))?;
                            let mask = parse_mask_opt(Some(mask_src), mask_exclude.as_deref())?
                                .unwrap_or_else(mcaedit_core::Mask::any);
                            let pattern =
                                parse_pattern_arg(pattern.as_deref(), with_block.as_deref())?;
                            world.replace_mask_pattern(
                                x1, y1, z1, x2, y2, z2, mask, pattern,
                            )?
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
                        EditCmd::Brush { cmd } => match cmd {
                            BrushCmd::Sphere {
                                at,
                                radius,
                                block,
                                pattern,
                                mask,
                                mask_exclude,
                                hollow,
                            } => {
                                let (cx, cy, cz) = parse_xyz_i(&at)?;
                                let pattern =
                                    parse_pattern_arg(pattern.as_deref(), block.as_deref())?;
                                let mask =
                                    parse_mask_opt(mask.as_deref(), mask_exclude.as_deref())?;
                                world.brush_sphere(
                                    cx, cy, cz, radius, pattern, mask, hollow,
                                )?
                            }
                            BrushCmd::Cyl {
                                at,
                                y,
                                radius,
                                height,
                                block,
                                pattern,
                                mask,
                                mask_exclude,
                                hollow,
                            } => {
                                let (cx, cz) = parse_xz(&at)?;
                                let pattern =
                                    parse_pattern_arg(pattern.as_deref(), block.as_deref())?;
                                let mask =
                                    parse_mask_opt(mask.as_deref(), mask_exclude.as_deref())?;
                                world.brush_cyl(
                                    cx, cz, y, radius, height, pattern, mask, hollow,
                                )?
                            }
                            BrushCmd::Clipboard {
                                at,
                                radius,
                                mask,
                                mask_exclude,
                            } => {
                                let (x, y, z) = parse_xyz_i(&at)?;
                                let mask =
                                    parse_mask_opt(mask.as_deref(), mask_exclude.as_deref())?;
                                world.brush_clipboard(x, y, z, radius, mask)?
                            }
                            BrushCmd::Biome {
                                at,
                                radius,
                                biome,
                                mask,
                                mask_exclude,
                                hollow,
                            } => {
                                let (cx, cy, cz) = parse_xyz_i(&at)?;
                                let mask =
                                    parse_mask_opt(mask.as_deref(), mask_exclude.as_deref())?;
                                world.brush_biome_sphere(
                                    cx, cy, cz, radius, &biome, mask, hollow,
                                )?
                            }
                            BrushCmd::BiomeCyl {
                                at,
                                y,
                                radius,
                                height,
                                biome,
                                mask,
                                mask_exclude,
                                hollow,
                            } => {
                                let (cx, cz) = parse_xz(&at)?;
                                let mask =
                                    parse_mask_opt(mask.as_deref(), mask_exclude.as_deref())?;
                                world.brush_biome_cyl(
                                    cx, cz, y, radius, height, &biome, mask, hollow,
                                )?
                            }
                        },
                        EditCmd::Smooth {
                            from,
                            to,
                            iterations,
                            kernel,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.smooth(x1, y1, z1, x2, y2, z2, iterations, kernel)?
                        }
                        EditCmd::Smooth3d {
                            from,
                            to,
                            iterations,
                            kernel,
                            solid,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.smooth3d(
                                x1, y1, z1, x2, y2, z2, iterations, kernel, solid,
                            )?
                        }
                        EditCmd::Biome { from, to, biome } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            world.biome_paint(x1, y1, z1, x2, y2, z2, &biome)?
                        }
                        EditCmd::Grid {
                            from,
                            to,
                            spacing_x,
                            spacing_z,
                            block,
                            pattern,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let pattern = parse_pattern_arg(pattern.as_deref(), block.as_deref())?;
                            mcaedit_core::build_ops::grid_columns(
                                &mut world,
                                x1,
                                y1,
                                z1,
                                x2,
                                y2,
                                z2,
                                spacing_x,
                                spacing_z,
                                pattern,
                            )?
                        }
                        EditCmd::RoofRows {
                            from,
                            to,
                            axis,
                            period,
                            block,
                            stairs,
                            stairs_facing,
                            stairs_half,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            let stairs = stairs
                                .as_deref()
                                .map(BlockState::parse)
                                .transpose()
                                .map_err(|e| anyhow::anyhow!(e))?;
                            let axis = axis
                                .chars()
                                .next()
                                .ok_or_else(|| anyhow::anyhow!("--axis required"))?;
                            mcaedit_core::build_ops::roof_rows(
                                &mut world,
                                x1,
                                y1,
                                z1,
                                x2,
                                y2,
                                z2,
                                axis,
                                period,
                                block,
                                stairs,
                                &stairs_facing,
                                &stairs_half,
                            )?
                        }
                        EditCmd::Stairs {
                            from,
                            to,
                            block,
                            facing,
                            half,
                            shape,
                        } => {
                            let (x1, y1, z1) = parse_xyz_i(&from)?;
                            let (x2, y2, z2) = parse_xyz_i(&to)?;
                            let block =
                                BlockState::parse(&block).map_err(|e| anyhow::anyhow!(e))?;
                            mcaedit_core::build_ops::stairs_fill(
                                &mut world, x1, y1, z1, x2, y2, z2, block, &facing, &half, &shape,
                            )?
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
                        | EditCmd::Tick { .. } => unreachable!(),
                        // Grid/RoofRows/Stairs handled above
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
            TemplateCmd::ExportSchem { name, out } => {
                let tpl = Template::load(&cwd, &name)?;
                let info = mcaedit_core::schem::export_template(&tpl, &out)?;
                for line in info.lines() {
                    println!("{line}");
                }
            }
            TemplateCmd::ImportSchem { file, name } => {
                let tpl = mcaedit_core::schem::import_to_template(&file, &name)?;
                let path = tpl.save_to_disk(&cwd)?;
                for line in tpl.brief_lines() {
                    println!("{line}");
                }
                println!("saved={}", path.display());
            }
        },
        Commands::Schem { cmd } => match cmd {
            SchemCmd::Export { from, to, out } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let world = WorldView::new(&mut s);
                let (x1, y1, z1) = parse_xyz_i(&from)?;
                let (x2, y2, z2) = parse_xyz_i(&to)?;
                let info = mcaedit_core::schem::export_aabb(
                    &world, x1, y1, z1, x2, y2, z2, &out,
                )?;
                for line in info.lines() {
                    println!("{line}");
                }
            }
            SchemCmd::Import { file, at } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let mut world = WorldView::new(&mut s);
                let (x, y, z) = parse_xyz_i(&at)?;
                let action = mcaedit_core::schem::import_paste(&mut world, &file, x, y, z)?;
                println!(
                    "action={} changed={} dirty=true desc={}",
                    action.id,
                    action.changed_count(),
                    action.description
                );
            }
            SchemCmd::Info { file } => {
                let info = mcaedit_core::schem::info(&file)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "path": info.path,
                            "version": info.version,
                            "width": info.width,
                            "height": info.height,
                            "length": info.length,
                            "palette_n": info.palette_n,
                        })
                    );
                } else {
                    for line in info.lines() {
                        println!("{line}");
                    }
                }
            }
        },
        Commands::Structure { cmd } => match cmd {
            StructureCmd::List { world } => {
                let world_path = if let Some(w) = world {
                    w
                } else {
                    open_session(&cwd, cli.session.as_deref())?
                        .meta
                        .source_world
                        .clone()
                };
                let list = mcaedit_core::structure::list_in_world(&world_path)?;
                println!("structures n={}", list.len());
                for p in list {
                    println!("{}", p.display());
                }
            }
            StructureCmd::Info { file } => {
                let info = mcaedit_core::structure::info(&file)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "path": info.path,
                            "size": info.size,
                            "blocks": info.blocks,
                            "entities": info.entities,
                            "palette": info.palette,
                            "DataVersion": info.data_version,
                            "author": info.author,
                        })
                    );
                } else {
                    for line in info.lines() {
                        println!("{line}");
                    }
                }
            }
            StructureCmd::Place {
                file,
                at,
                rotation,
                mirror,
                no_entities,
            } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let mut world = WorldView::new(&mut s);
                let (x, y, z) = parse_xyz_i(&at)?;
                let mirror = match mirror.as_deref() {
                    None => None,
                    Some("x") | Some("X") => Some('x'),
                    Some("z") | Some("Z") => Some('z'),
                    Some(other) => bail!("mirror must be x or z, got `{other}`"),
                };
                let action = mcaedit_core::structure::place(
                    &mut world,
                    &file,
                    x,
                    y,
                    z,
                    mcaedit_core::structure::PlaceOptions {
                        rotation,
                        mirror,
                        include_entities: !no_entities,
                    },
                )?;
                println!(
                    "action={} changed={} dirty=true desc={}",
                    action.id,
                    action.changed_count(),
                    action.description
                );
            }
            StructureCmd::Export {
                from,
                to,
                out,
                data_version,
            } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let dv = data_version
                    .or(s.meta.data_version)
                    .unwrap_or(mcaedit_core::mc_version::DEFAULT_DATA_VERSION);
                let world = WorldView::new(&mut s);
                let (x1, y1, z1) = parse_xyz_i(&from)?;
                let (x2, y2, z2) = parse_xyz_i(&to)?;
                let info = mcaedit_core::structure::export_aabb(
                    &world,
                    [x1, y1, z1],
                    [x2, y2, z2],
                    &out,
                    dv,
                )?;
                for line in info.lines() {
                    println!("{line}");
                }
            }
            StructureCmd::Import { file, name } => {
                let tpl = mcaedit_core::structure::import_to_template(&file, &name)?;
                let path = tpl.save_to_disk(&cwd)?;
                for line in tpl.brief_lines() {
                    println!("{line}");
                }
                println!("saved={}", path.display());
            }
            StructureCmd::ClearRefs {
                from,
                to,
                no_starts,
                no_references,
            } => {
                let mut s = open_session(&cwd, cli.session.as_deref())?;
                let mut world = WorldView::new(&mut s);
                let (x1, y1, z1) = parse_xyz_i(&from)?;
                let (x2, y2, z2) = parse_xyz_i(&to)?;
                let starts = !no_starts;
                let references = !no_references;
                let n = mcaedit_core::structure::clear_structures_in_aabb(
                    &mut world,
                    [x1, y1, z1],
                    [x2, y2, z2],
                    starts,
                    references,
                )?;
                println!("cleared_chunks={n} starts={starts} references={references}");
            }
        },
        Commands::World { cmd } => match cmd {
            WorldCmd::Create {
                path,
                level_name,
                seed,
                spawn,
                game_type,
                generator,
                mc,
                data_version,
                region_format,
                all_dims,
                force,
            } => {
                let (sx, sy, sz) = parse_xyz_i(&spawn)?;
                let version =
                    mcaedit_core::mc_version::ResolvedVersion::resolve(Some(&mc), data_version)?;
                let opts = mcaedit_core::level::WorldCreateOptions {
                    path,
                    level_name,
                    seed,
                    spawn: [sx, sy, sz],
                    game_type,
                    generator: mcaedit_core::level::GeneratorKind::parse(&generator)?,
                    version,
                    region_format: mcaedit_core::level::RegionFormat::parse(&region_format)?,
                    all_dims,
                    force,
                };
                let info = mcaedit_core::level::create_world(&opts)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "path": info.path,
                            "LevelName": info.level_name,
                            "Seed": info.seed,
                            "Spawn": info.spawn,
                            "GameType": info.game_type,
                            "DataVersion": info.data_version,
                            "Version.Name": info.version_name,
                            "modern_layout": info.modern_layout,
                        })
                    );
                } else {
                    for line in info.lines() {
                        println!("{line}");
                    }
                    println!("created=true");
                }
            }
        },
        Commands::Level { cmd } => match cmd {
            LevelCmd::Info { world } => {
                let info = mcaedit_core::level::info(&world)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "path": info.path,
                            "LevelName": info.level_name,
                            "Seed": info.seed,
                            "Spawn": info.spawn,
                            "GameType": info.game_type,
                            "DataVersion": info.data_version,
                            "Version.Name": info.version_name,
                            "LastPlayed": info.last_played,
                            "generator": info.generator,
                            "modern_layout": info.modern_layout,
                        })
                    );
                } else {
                    for line in info.lines() {
                        println!("{line}");
                    }
                }
            }
            LevelCmd::Write {
                world,
                level_name,
                seed,
                spawn,
                game_type,
                mc,
                data_version,
                generator,
                force,
            } => {
                let version = mcaedit_core::mc_version::ResolvedVersion::resolve(
                    mc.as_deref(),
                    data_version,
                )?;
                let spawn = if let Some(s) = spawn {
                    let (x, y, z) = parse_xyz_i(&s)?;
                    [x, y, z]
                } else {
                    [0, 64, 0]
                };
                let opts = mcaedit_core::level::WorldCreateOptions {
                    path: world,
                    level_name: level_name.unwrap_or_else(|| "world".into()),
                    seed: seed.unwrap_or(0),
                    spawn,
                    game_type: game_type.unwrap_or(1),
                    generator: mcaedit_core::level::GeneratorKind::parse(&generator)?,
                    version,
                    region_format: mcaedit_core::level::RegionFormat::Anvil,
                    all_dims: false,
                    force,
                };
                let info = mcaedit_core::level::create_world(&opts)?;
                for line in info.lines() {
                    println!("{line}");
                }
                println!("written=true");
            }
            LevelCmd::Patch {
                world,
                level_name,
                seed,
                spawn,
                game_type,
                data_version,
                version_name,
                touch,
            } => {
                let spawn = if let Some(s) = spawn {
                    let (x, y, z) = parse_xyz_i(&s)?;
                    Some([x, y, z])
                } else {
                    None
                };
                let info = mcaedit_core::level::update_level_dat(
                    &world,
                    &mcaedit_core::level::LevelPatchOptions {
                        level_name: level_name.as_deref(),
                        seed,
                        spawn,
                        game_type,
                        data_version,
                        version_name: version_name.as_deref(),
                        touch_last_played: touch,
                    },
                )?;
                for line in info.lines() {
                    println!("{line}");
                }
                println!("patched=true");
            }
        },
        Commands::View { cmd } => {
            let mut s = open_session(&cwd, cli.session.as_deref())?;
            match cmd {
                ViewCmd::Screenshot {
                    from,
                    to,
                    out,
                    width,
                    height,
                    camera,
                    look,
                    minecraft,
                    assets_jar,
                    no_textures,
                    max_cells,
                } => {
                    let world = WorldView::new(&mut s);
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
                        minecraft,
                        assets_jar,
                        no_textures,
                        max_cells,
                    };
                    let rep = world.view_screenshot(&req)?;
                    println!(
                        "screenshot={} size={}x{} blocks={} faces={} triangles={} textures={}",
                        rep.out.display(),
                        rep.width,
                        rep.height,
                        rep.blocks,
                        rep.faces,
                        rep.triangles,
                        rep.textures
                    );
                }
                ViewCmd::Preview {
                    from,
                    to,
                    watch,
                    width,
                    height,
                    minecraft,
                    assets_jar,
                    no_textures,
                    max_cells,
                } => {
                    run_preview_cmd(
                        &cwd,
                        &s,
                        PreviewLaunch {
                            from,
                            to,
                            watch,
                            width,
                            height,
                            minecraft,
                            assets_jar,
                            no_textures,
                            max_cells,
                        },
                    )?;
                }
            }
        }
        Commands::Preview {
            from,
            to,
            watch,
            width,
            height,
            minecraft,
            assets_jar,
            no_textures,
            max_cells,
        } => {
            let s = open_session(&cwd, cli.session.as_deref())?;
            run_preview_cmd(
                &cwd,
                &s,
                PreviewLaunch {
                    from,
                    to,
                    watch,
                    width,
                    height,
                    minecraft,
                    assets_jar,
                    no_textures,
                    max_cells,
                },
            )?;
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

struct PreviewLaunch {
    from: Option<String>,
    to: Option<String>,
    watch: u64,
    width: u32,
    height: u32,
    minecraft: Option<PathBuf>,
    assets_jar: Option<PathBuf>,
    no_textures: bool,
    max_cells: Option<usize>,
}

fn run_preview_cmd(cwd: &std::path::Path, session: &Session, launch: PreviewLaunch) -> Result<()> {
    #[cfg(not(feature = "preview"))]
    {
        let _ = (cwd, session, launch);
        bail!(
            "preview feature disabled; rebuild with `--features preview` (default on release builds)"
        );
    }
    #[cfg(feature = "preview")]
    {
        let pinned_from = launch.from.as_deref().map(parse_xyz_i).transpose()?;
        let pinned_to = launch.to.as_deref().map(parse_xyz_i).transpose()?;
        if pinned_from.is_some() != pinned_to.is_some() {
            bail!("preview needs both --from and --to, or neither (auto AABB)");
        }
        let (show_from, show_to) = match (pinned_from, pinned_to) {
            (Some(f), Some(t)) => (f, t),
            _ => suggest_preview_aabb(session)?,
        };
        let (_models_probe, textures_label) = mcaedit_core::view::resolve_models_cli(
            launch.minecraft.as_deref(),
            launch.assets_jar.as_deref(),
            launch.no_textures,
        );
        println!(
            "preview session={} aabb=({},{},{})..({},{},{}) watch={}ms textures={} (needs DISPLAY/Wayland)",
            session.meta.id,
            show_from.0,
            show_from.1,
            show_from.2,
            show_to.0,
            show_to.1,
            show_to.2,
            launch.watch,
            textures_label
        );
        preview::run_preview(preview::PreviewOptions {
            session_id: session.meta.id.clone(),
            cwd: cwd.to_path_buf(),
            from: pinned_from,
            to: pinned_to,
            watch_ms: launch.watch,
            width: launch.width,
            height: launch.height,
            minecraft: launch.minecraft,
            assets_jar: launch.assets_jar,
            no_textures: launch.no_textures,
            max_cells: launch.max_cells,
        })?;
        Ok(())
    }
}

fn print_session(s: &Session, json: bool) {
    if json {
        println!("{}", serde_json::to_string(&s.meta).unwrap());
    } else {
        for line in s.status_lines() {
            println!("{line}");
        }
        if let Some(note) = &s.bootstrap_note {
            println!("{note}");
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

fn parse_pattern_arg(pattern: Option<&str>, block: Option<&str>) -> Result<mcaedit_core::Pattern> {
    let src = pattern
        .or(block)
        .ok_or_else(|| anyhow::anyhow!("need --block or --pattern"))?;
    mcaedit_core::Pattern::parse(src).map_err(|e| anyhow::anyhow!(e))
}

fn parse_mask_opt(
    mask: Option<&str>,
    mask_exclude: Option<&str>,
) -> Result<Option<mcaedit_core::Mask>> {
    if mask.is_none() && mask_exclude.is_none() {
        return Ok(None);
    }
    let base = match mask {
        Some(s) => Some(mcaedit_core::Mask::parse(s).map_err(|e| anyhow::anyhow!(e))?),
        None => None,
    };
    Ok(Some(
        mcaedit_core::Mask::with_exclude(base, mask_exclude)
            .map_err(|e| anyhow::anyhow!(e))?,
    ))
}

#[cfg(test)]
mod clap_coord_tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn accepts_negative_from_with_space() {
        let cli = Cli::try_parse_from([
            "mcaedit",
            "edit",
            "fill",
            "--from",
            "-8,60,-8",
            "--to",
            "8,62,8",
            "--block",
            "minecraft:stone",
        ]);
        assert!(cli.is_ok(), "{cli:?}");
    }

    #[test]
    fn accepts_negative_from_equals_form() {
        let cli = Cli::try_parse_from([
            "mcaedit",
            "view",
            "screenshot",
            "--from=-10,55,-55",
            "--to=45,100,12",
            "--camera",
            "-28,105,-78",
            "--look=16,78,-22",
            "--out",
            "/tmp/x.png",
        ]);
        assert!(cli.is_ok(), "{cli:?}");
    }

    #[test]
    fn accepts_negative_chunk_xz_for_fix_light() {
        let cli = Cli::try_parse_from([
            "mcaedit",
            "edit",
            "fix-light",
            "--from",
            "-2,-2",
            "--to",
            "1,1",
        ]);
        assert!(cli.is_ok(), "{cli:?}");
    }
}
