//! The web build's loading screen (`#tbs-loading` in `index.html`). The
//! page shows it, with download progress, while the wasm downloads
//! (`loader.js`). The game then shows what it's doing ("Loading
//! missions...", "Building the level...") and fades the screen out once the
//! first mission has had a few frames to draw. Other platforms start fast
//! enough not to need one: the plugin does nothing there.

use bevy::prelude::*;

pub struct LoadingScreenPlugin;

impl Plugin for LoadingScreenPlugin {
    fn build(&self, _app: &mut App) {
        #[cfg(target_arch = "wasm32")]
        _app.add_systems(Update, web::update);
    }
}

/// Frames the first mission gets to draw before the screen fades out.
pub const SETTLE_FRAMES: u32 = 12;

/// What the loading screen says, from where the game is.
pub fn stage_text(missions_ready: bool, mission_started: bool) -> &'static str {
    match (missions_ready, mission_started) {
        (false, _) => "Loading missions and modules...",
        (true, false) => "Preparing the battlefield...",
        (true, true) => "Building the level...",
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use bevy::prelude::*;

    use crate::missions::MissionCatalog;

    #[derive(Default)]
    pub(super) struct Progress {
        frames: u32,
        last: &'static str,
        done: bool,
        /// Seconds since the fade started (the element is removed after it).
        fading: Option<f32>,
    }

    pub(super) fn update(catalog: Option<Res<MissionCatalog>>, time: Res<Time<Real>>, mut p: Local<Progress>) {
        if p.done {
            return;
        }
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(screen) = doc.get_element_by_id("tbs-loading") else {
            p.done = true;
            return;
        };
        if let Some(t) = p.fading.as_mut() {
            *t += time.delta_secs();
            if *t > 0.6 {
                screen.remove();
                p.done = true;
            }
            return;
        }
        let ready = catalog.as_ref().is_some_and(|c| c.ready);
        let started = catalog.as_ref().is_some_and(|c| c.started());
        let text = super::stage_text(ready, started);
        if text != p.last {
            p.last = text;
            if let Some(status) = doc.get_element_by_id("tbs-loading-status") {
                status.set_text_content(Some(text));
            }
            if let Some(bar) = doc.get_element_by_id("tbs-loading-bar") {
                let _ = bar.set_attribute("class", "indeterminate");
            }
        }
        if started {
            p.frames += 1;
            if p.frames >= super::SETTLE_FRAMES {
                let _ = screen.set_attribute("class", "done");
                p.fading = Some(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_follow_loading() {
        assert!(stage_text(false, false).contains("missions"));
        assert!(stage_text(true, false).contains("battlefield"));
        assert!(stage_text(true, true).contains("level"));
    }
}
