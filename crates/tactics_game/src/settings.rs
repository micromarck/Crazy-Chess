//! Player settings: animation speed, camera feel, edge scrolling.
//!
//! Changed from the HUD's Settings panel with `setting:<name>` commands and
//! saved (desktop) to `tbsium_settings.txt` in the working directory as
//! simple `key = value` lines.
//!
//! | setting | values | effect |
//! |---|---|---|
//! | `anim_speed` | 1, 1.5, 2, 3 | playback speed (Shift still fast-forwards x3 on top) |
//! | `camera_speed` | 0.6, 1, 1.5 | keyboard pan / rotate / zoom speed and the focus glide |
//! | `edge_scroll` | on/off | pan when the mouse touches the window edge |
//! | `invert_orbit` | on/off | flip right-drag tilt direction |
//! | `action_camera` | on/off | frame shooter and target on attacks |
//! | `ui_scale` | auto, 0.7 - 1.5 | size of the HUD and menus (auto: a little smaller on phones, see [`crate::touch`]) |
//! | `difficulty` | easy, normal, hard | enemy AI: easy = no threat reading, flanking or focus fire; hard = braver and bolder |

use bevy::prelude::*;

use crate::FrameSet;
use crate::camera::CameraRig;
use crate::camera_director::CameraDirector;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GameSettings::load()).add_systems(Update, edge_scroll.in_set(FrameSet::Input));
    }
}

/// Everything the Settings panel controls.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameSettings {
    pub anim_speed: f32,
    pub camera_speed: f32,
    pub edge_scroll: bool,
    pub invert_orbit: bool,
    pub action_camera: bool,
    /// 0 = easy, 1 = normal, 2 = hard.
    pub difficulty: u8,
    /// Soldiers who fall are lost for good (roster).
    pub permadeath: bool,
    /// HUD / menu scale; 0 = automatic (see [`crate::touch`]).
    pub ui_scale: f32,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            anim_speed: 1.0,
            camera_speed: 1.0,
            edge_scroll: false,
            invert_orbit: false,
            action_camera: true,
            difficulty: 1,
            permadeath: false,
            ui_scale: 0.0,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
const FILE: &str = "tbsium_settings.txt";

fn next_of(options: &[f32], current: f32) -> f32 {
    let i = options.iter().position(|o| (o - current).abs() < 1e-3).map_or(0, |i| (i + 1) % options.len());
    options[i]
}

impl GameSettings {
    /// Applies a `setting:<name>` command (cycles / toggles). Returns a
    /// description for the log.
    pub fn command(&mut self, name: &str) -> Option<String> {
        let msg = match name.trim() {
            "anim_speed" => {
                self.anim_speed = next_of(&[1.0, 1.5, 2.0, 3.0], self.anim_speed);
                format!("Animation speed x{}", self.anim_speed)
            }
            "camera_speed" => {
                self.camera_speed = next_of(&[0.6, 1.0, 1.5], self.camera_speed);
                format!("Camera speed x{}", self.camera_speed)
            }
            "edge_scroll" => {
                self.edge_scroll = !self.edge_scroll;
                format!("Edge scrolling {}", on_off(self.edge_scroll))
            }
            "invert_orbit" => {
                self.invert_orbit = !self.invert_orbit;
                format!("Inverted tilt {}", on_off(self.invert_orbit))
            }
            "action_camera" => {
                self.action_camera = !self.action_camera;
                format!("Action camera {}", on_off(self.action_camera))
            }
            "permadeath" => {
                self.permadeath = !self.permadeath;
                format!("Permadeath {}", on_off(self.permadeath))
            }
            "ui_scale" => {
                self.ui_scale = next_of(&[0.0, 0.7, 0.85, 1.0, 1.15, 1.3, 1.5], self.ui_scale);
                format!("UI size: {}", self.ui_scale_name())
            }
            "difficulty" => {
                self.difficulty = (self.difficulty + 1) % 3;
                format!("AI difficulty: {}", self.difficulty_name())
            }
            _ => return None,
        };
        self.save();
        Some(msg)
    }

    pub fn to_text(&self) -> String {
        format!(
            "anim_speed = {}\ncamera_speed = {}\nedge_scroll = {}\ninvert_orbit = {}\naction_camera = {}\ndifficulty = {}\npermadeath = {}\nui_scale = {}\n",
            self.anim_speed, self.camera_speed, self.edge_scroll, self.invert_orbit, self.action_camera, self.difficulty, self.permadeath, self.ui_scale
        )
    }

    /// "auto" or "x1.15".
    pub fn ui_scale_name(&self) -> String {
        if self.ui_scale <= 0.0 { "auto".into() } else { format!("x{}", self.ui_scale) }
    }

    pub fn difficulty_name(&self) -> &'static str {
        ["Easy", "Normal", "Hard"][self.difficulty.min(2) as usize]
    }

    /// The enemy AI tuning for the chosen difficulty, starting from `base`.
    pub fn ai_config(&self, base: tactics_core::ai::AiConfig) -> tactics_core::ai::AiConfig {
        match self.difficulty {
            0 => tactics_core::ai::AiConfig {
                danger_weight: 0.0,
                flank_bonus: 0.0,
                focus_fire_weight: 0.0,
                threat_weight: 0.0,
                cluster_penalty: 0.0,
                shoot_threshold: base.shoot_threshold + 5,
                ..base
            },
            2 => tactics_core::ai::AiConfig {
                fear_response: base.fear_response * 0.5,
                confident_below: base.confident_below + 25,
                shoot_threshold: base.shoot_threshold - 5,
                flank_bonus: base.flank_bonus * 1.5,
                ..base
            },
            _ => base,
        }
    }

    /// Parses `key = value` lines; unknown or bad lines are ignored.
    pub fn from_text(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let f = v.parse::<f32>().ok().filter(|x| x.is_finite() && *x > 0.0 && *x <= 5.0);
            let b = v.parse::<bool>().ok();
            match k {
                "anim_speed" => s.anim_speed = f.unwrap_or(s.anim_speed),
                "camera_speed" => s.camera_speed = f.unwrap_or(s.camera_speed),
                "edge_scroll" => s.edge_scroll = b.unwrap_or(s.edge_scroll),
                "invert_orbit" => s.invert_orbit = b.unwrap_or(s.invert_orbit),
                "action_camera" => s.action_camera = b.unwrap_or(s.action_camera),
                "permadeath" => s.permadeath = b.unwrap_or(s.permadeath),
                "ui_scale" => s.ui_scale = v.parse::<f32>().ok().filter(|x| *x == 0.0 || (0.5..=2.0).contains(x)).unwrap_or(s.ui_scale),
                "difficulty" => s.difficulty = v.parse::<u8>().ok().filter(|d| *d <= 2).unwrap_or(s.difficulty),
                _ => {}
            }
        }
        s
    }

    fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(Ok(text)) = crate::save_path(FILE).map(std::fs::read_to_string) {
            return Self::from_text(&text);
        }
        Self::default()
    }

    fn save(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(Err(e)) = crate::save_path(FILE).map(|path| std::fs::write(path, self.to_text())) {
            warn!("couldn't save settings: {e}");
        }
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// Pans when the mouse rests at the window edge (if enabled).
fn edge_scroll(
    time: Res<Time<Real>>,
    settings: Res<GameSettings>,
    director: Res<CameraDirector>,
    window: Single<&Window>,
    mut rig: Single<&mut CameraRig>,
) {
    if !settings.edge_scroll || director.is_active() || !window.focused {
        return;
    }
    let Some(c) = window.cursor_position() else { return };
    let (w, h) = (window.width(), window.height());
    const EDGE: f32 = 6.0;
    let mut dir = Vec2::ZERO;
    if c.x <= EDGE {
        dir.x -= 1.0;
    }
    if c.x >= w - EDGE {
        dir.x += 1.0;
    }
    if c.y <= EDGE {
        dir.y += 1.0;
    }
    if c.y >= h - EDGE {
        dir.y -= 1.0;
    }
    if dir != Vec2::ZERO {
        let speed = rig.target_distance * 1.2 * settings.camera_speed * time.delta_secs();
        rig.pan_by(dir.normalize() * speed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_cycling() {
        let s = GameSettings { anim_speed: 2.0, edge_scroll: true, ..Default::default() };
        assert_eq!(GameSettings::from_text(&s.to_text()), s);
        assert_eq!(GameSettings::from_text("anim_speed = banana\nnonsense\nedge_scroll = true"), GameSettings { edge_scroll: true, ..Default::default() });
        let mut t = GameSettings { anim_speed: 3.0, ..Default::default() };
        t.anim_speed = next_of(&[1.0, 1.5, 2.0, 3.0], t.anim_speed);
        assert_eq!(t.anim_speed, 1.0, "wraps around");
    }
}
