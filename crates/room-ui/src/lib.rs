//! Shared presentation layer. It owns viewer state and emits typed platform intents.
//! Capture, codec, texture lifetime and room membership stay in their existing adapters.
mod mobile;
mod language;
mod mobile_volume;
pub mod mobile_profiles;
mod mobile_navigation;
mod mobile_pages;
mod mobile_overlay;
mod audio;
mod chrome;
mod contracts;
mod dialogs;
mod layout;
mod members;
mod model;
pub use model::{DiagnosticRun, DiagnosticReceipt};
mod streams;
mod theme;
mod video;
mod viewer;
pub use contracts::{MemberState, RoomState};
pub use layout::Platform;
pub use model::{Action, Participant, RoomUi, UiSnapshot, peer_ui_id};
impl RoomUi {
    pub fn ui(&mut self, root: &mut egui::Ui) -> Vec<Action> {
        let _language=p2pshare_media::i18n::scope(self.language);
        if self.language_prompt{return self.language_dialog(root);}
        let name_before = self.name.clone();
        let watching_before = self.watching.clone();
        let mut actions = Vec::new();
        self.snapshot = UiSnapshot::default();
        self.profiles.hits.clear();
        self.reconcile_focus();
        let ctx = root.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.back();
        }
        let layout = layout::Layout::new(root.max_rect(), self.platform);
        self.snapshot.mobile = layout.mobile;
        if let Some(id) = self.focused
            && self
                .participants
                .iter()
                .any(|p| p.id == id && p.sharing && !p.local)
        {
            self.watching.insert(id);
        }
        if self.pip_mode {
            self.pip_view(root);
        } else if self.is_fullscreen() {
            if self.platform == Platform::Android {self.mobile_fullscreen(root,&mut actions);}else{self.fullscreen_view(root);}
        } else {
            let safe = layout::safe_rect(root.max_rect(), self.safe_insets);
            root.scope_builder(egui::UiBuilder::new().max_rect(safe), |ui| {
                if self.platform == Platform::Android {
                    self.mobile_ui(ui, &layout, &mut actions);
                } else {
                    self.chrome(ui, &layout, &mut actions);
                    self.streams(ui, &layout, &mut actions);
                }
            });
            if self.platform != Platform::Android { self.dialogs(&ctx, layout.mobile, &mut actions); }
        }
        if self.platform == Platform::Android{self.mobile_volume_dialog(&ctx);}
        if self.platform == Platform::Android && !self.pip_mode{self.mobile_profile_dialogs(&ctx,&mut actions);}
        self.snapshot.fullscreen = self.is_fullscreen();
        if name_before != self.name {
            actions.push(Action::Rename(self.name.clone()));
        }
        actions.extend(
            self.audio
                .pending
                .drain(..)
                .map(|(participant, muted)| Action::AudioMute { participant, muted }),
        );
        actions.extend(self.audio.volumes.drain(..).map(|(participant, percent)| {
            Action::AudioVolume {
                participant,
                percent,
            }
        }));
        let mode = (self.is_fullscreen(), self.handles_back());
        if mode != self.last_viewer_mode {
            self.last_viewer_mode = mode;
            actions.push(Action::ViewerMode {
                fullscreen: mode.0,
                handles_back: mode.1,
            });
        }
        for id in watching_before.union(&self.watching) {
            if watching_before.contains(id) != self.watching.contains(id) {
                actions.push(Action::Watch {
                    participant: *id,
                    enabled: self.watching.contains(id),
                });
            }
        }
        actions
    }
}
