//! Live session preview window (eframe/egui + shared soft raster mesh).

use anyhow::{Context, Result};
#[cfg(all(unix, not(target_os = "macos")))]
use anyhow::bail;
use eframe::egui;
use glam::Vec3;
use mcaedit_core::models::ModelCatalog;
use mcaedit_core::session::Session;
use mcaedit_core::view::{
    default_camera_for_mesh, orbit_camera, preview_watch_token, rasterize_view_mesh,
    resolve_models_cli, suggest_preview_aabb, PreviewWatchToken, RgbaFrame, ViewMesh,
};
use mcaedit_core::world::WorldView;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct PreviewOptions {
    pub session_id: String,
    pub cwd: PathBuf,
    pub from: Option<(i32, i32, i32)>,
    pub to: Option<(i32, i32, i32)>,
    pub watch_ms: u64,
    pub width: u32,
    pub height: u32,
    pub minecraft: Option<PathBuf>,
    pub assets_jar: Option<PathBuf>,
    pub no_textures: bool,
    pub max_cells: Option<usize>,
}

pub fn run_preview(opts: PreviewOptions) -> Result<()> {
    // Linux X11/Wayland need an explicit display; macOS/Windows use the native GUI stack.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
        && std::env::var_os("MCAEDIT_PREVIEW_ALLOW_HEADLESS").is_none()
    {
        bail!(
            "preview needs a display (DISPLAY or WAYLAND_DISPLAY). \
             Set MCAEDIT_PREVIEW_ALLOW_HEADLESS=1 only for debugging."
        );
    }

    let session = Session::open(&opts.cwd, &opts.session_id)
        .context("open session for preview")?;
    let (from, to) = match (opts.from, opts.to) {
        (Some(f), Some(t)) => (f, t),
        _ => suggest_preview_aabb(&session)?,
    };

    let mut native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([opts.width as f32, opts.height as f32])
            .with_title(format!("MCAEdit preview — {}", opts.session_id)),
        ..Default::default()
    };
    // Safety net if preview is ever started off the OS main thread (Linux only).
    // Prefix `_builder` so macOS/Windows clippy stays clean under `-D warnings`.
    native_options.event_loop_builder = Some(Box::new(|_builder| {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                use winit::platform::wayland::EventLoopBuilderExtWayland;
                EventLoopBuilderExtWayland::with_any_thread(_builder, true);
            } else {
                use winit::platform::x11::EventLoopBuilderExtX11;
                EventLoopBuilderExtX11::with_any_thread(_builder, true);
            }
        }
    }));

    let app = PreviewApp::new(opts, session, from, to)?;
    eframe::run_native(
        "MCAEdit Preview",
        native_options,
        Box::new(|_cc| Ok(Box::new(app))),
    )
    .map_err(|e| anyhow::anyhow!("preview window failed: {e}"))?;
    Ok(())
}

struct PreviewApp {
    opts: PreviewOptions,
    session_root: PathBuf,
    from: (i32, i32, i32),
    to: (i32, i32, i32),
    mesh: ViewMesh,
    catalog: Option<ModelCatalog>,
    textures_label: String,
    token: PreviewWatchToken,
    last_reload: Instant,
    last_reload_unix: u64,
    yaw: f32,
    pitch: f32,
    distance: f32,
    look: Vec3,
    pan: Vec3,
    frame: Option<RgbaFrame>,
    texture: Option<egui::TextureHandle>,
    camera_dirty: bool,
    status: String,
    error: Option<String>,
    auto_orbit: bool,
}

impl PreviewApp {
    fn new(
        opts: PreviewOptions,
        mut session: Session,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> Result<Self> {
        let session_root = session.root.clone();
        let token = preview_watch_token(&session_root)?;
        let (catalog, textures_label) = resolve_models_cli(
            opts.minecraft.as_deref(),
            opts.assets_jar.as_deref(),
            opts.no_textures,
        );
        let mut catalog = catalog;
        let world = WorldView::new(&mut session);
        let mesh = world
            .build_view_mesh_with_textures_limited(from, to, catalog.as_mut(), opts.max_cells)
            .context("initial mesh build")?;
        let (look, _cam) = default_camera_for_mesh(&mesh, None, None);
        let span = {
            let sx = (mesh.to.0 - mesh.from.0 + 1) as f32;
            let sy = (mesh.to.1 - mesh.from.1 + 1) as f32;
            let sz = (mesh.to.2 - mesh.from.2 + 1) as f32;
            sx.max(sy).max(sz).max(8.0)
        };
        let mut app = Self {
            opts,
            session_root,
            from,
            to,
            mesh,
            catalog,
            textures_label,
            token,
            last_reload: Instant::now(),
            last_reload_unix: now_unix(),
            yaw: 35.0,
            pitch: 28.0,
            distance: span * 1.6,
            look,
            pan: Vec3::ZERO,
            frame: None,
            texture: None,
            camera_dirty: true,
            status: String::new(),
            error: None,
            auto_orbit: true,
        };
        app.rerender(640, 360);
        Ok(app)
    }

    fn poll_reload(&mut self) {
        let interval = Duration::from_millis(self.opts.watch_ms.max(50));
        if self.last_reload.elapsed() < interval {
            return;
        }
        self.last_reload = Instant::now();
        match preview_watch_token(&self.session_root) {
            Ok(token) if token != self.token => {
                self.token = token;
                if let Err(e) = self.rebuild_mesh() {
                    self.error = Some(e.to_string());
                } else {
                    self.error = None;
                    self.camera_dirty = true;
                    self.last_reload_unix = now_unix();
                    self.status = format!(
                        "reloaded head={}/{} dirty={}",
                        self.token.head, self.token.history_size, self.token.dirty
                    );
                }
            }
            Ok(_) => {}
            Err(e) => self.error = Some(format!("watch: {e}")),
        }
    }

    fn rebuild_mesh(&mut self) -> Result<()> {
        let mut session = Session::open(&self.opts.cwd, &self.opts.session_id)?;
        // Refresh AABB if user did not pin one (new chunks may appear).
        if self.opts.from.is_none() || self.opts.to.is_none() {
            let (f, t) = suggest_preview_aabb(&session)?;
            self.from = f;
            self.to = t;
        }
        let world = WorldView::new(&mut session);
        self.mesh = world.build_view_mesh_with_textures_limited(
            self.from,
            self.to,
            self.catalog.as_mut(),
            self.opts.max_cells,
        )?;
        let (look, _) = default_camera_for_mesh(&self.mesh, None, None);
        self.look = look;
        Ok(())
    }

    fn camera_pos(&self) -> Vec3 {
        orbit_camera(self.look + self.pan, self.yaw, self.pitch, self.distance)
    }

    fn rerender(&mut self, width: u32, height: u32) {
        let w = width.clamp(64, 1920);
        let h = height.clamp(64, 1080);
        let cam = self.camera_pos();
        self.frame = Some(rasterize_view_mesh(
            &self.mesh,
            w,
            h,
            cam,
            self.look + self.pan,
        ));
        self.camera_dirty = false;
    }
}

impl eframe::App for PreviewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_reload();
        if self.auto_orbit {
            self.yaw = (self.yaw + ctx.input(|i| i.stable_dt) * 12.0) % 360.0;
            self.camera_dirty = true;
        }

        egui::TopBottomPanel::top("hud").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "session={}  aabb=({},{},{})..({},{},{})",
                    self.opts.session_id,
                    self.from.0,
                    self.from.1,
                    self.from.2,
                    self.to.0,
                    self.to.1,
                    self.to.2
                ));
                ui.separator();
                ui.label(format!(
                    "blocks={} faces={} tris={} tex={}",
                    self.mesh.blocks,
                    self.mesh.faces,
                    self.mesh.triangles.len(),
                    if self.textures_label == "palette" {
                        "palette"
                    } else {
                        "jar"
                    }
                ));
                ui.separator();
                ui.label(format!(
                    "hist={}/{} dirty={}  reload_t={}",
                    self.token.head,
                    self.token.history_size,
                    self.token.dirty,
                    self.last_reload_unix
                ));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.auto_orbit, "auto-orbit");
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
            });
            ui.label("LMB drag: orbit · RMB/MMB drag: pan · wheel: zoom · Space: toggle auto-orbit");
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let avail = ui.available_size();
            let w = avail.x.max(64.0) as u32;
            let h = avail.y.max(64.0) as u32;

            let response = ui.allocate_response(avail, egui::Sense::click_and_drag());
            let input = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Space),
                    i.raw_scroll_delta.y,
                    i.pointer.button_down(egui::PointerButton::Primary),
                    i.pointer.button_down(egui::PointerButton::Secondary)
                        || i.pointer.button_down(egui::PointerButton::Middle),
                    i.pointer.delta(),
                )
            });
            if input.0 {
                self.auto_orbit = !self.auto_orbit;
            }
            if input.1.abs() > 0.0 {
                let zoom = 1.0 - input.1 * 0.0015;
                self.distance = (self.distance * zoom).clamp(4.0, 512.0);
                self.camera_dirty = true;
            }
            if response.dragged() {
                let delta = input.4;
                if input.2 {
                    self.yaw = (self.yaw + delta.x * 0.35) % 360.0;
                    self.pitch = (self.pitch - delta.y * 0.25).clamp(-85.0, 85.0);
                    self.camera_dirty = true;
                    self.auto_orbit = false;
                } else if input.3 {
                    let right = Vec3::new(self.yaw.to_radians().cos(), 0.0, self.yaw.to_radians().sin());
                    let up = Vec3::Y;
                    let scale = self.distance * 0.0025;
                    self.pan += right * (-delta.x * scale) + up * (delta.y * scale);
                    self.camera_dirty = true;
                    self.auto_orbit = false;
                }
            }

            if self.camera_dirty || self.frame.as_ref().is_none_or(|f| f.width != w || f.height != h)
            {
                self.rerender(w, h);
            }

            if let Some(frame) = &self.frame {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.pixels,
                );
                let tex = self.texture.get_or_insert_with(|| {
                    ctx.load_texture("preview-frame", color.clone(), Default::default())
                });
                tex.set(color, Default::default());
                let size = egui::vec2(frame.width as f32, frame.height as f32);
                ui.put(
                    egui::Rect::from_min_size(response.rect.min, size),
                    egui::Image::new(egui::load::SizedTexture::new(tex.id(), size)),
                );
            }
        });

        ctx.request_repaint_after(Duration::from_millis(self.opts.watch_ms.clamp(16, 100)));
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::PreviewOptions;
    use std::path::PathBuf;

    #[test]
    fn preview_options_carry_watch_interval() {
        let opts = PreviewOptions {
            session_id: "t".into(),
            cwd: PathBuf::from("."),
            from: None,
            to: None,
            watch_ms: 400,
            width: 960,
            height: 540,
            minecraft: None,
            assets_jar: None,
            no_textures: false,
            max_cells: None,
        };
        assert_eq!(opts.watch_ms, 400);
        assert!(opts.from.is_none());
    }
}
