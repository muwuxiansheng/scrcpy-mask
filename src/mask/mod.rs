pub mod mapping;
pub mod mask_command;
pub mod ui;
pub mod video;

use std::time::Duration;

use bevy::{
    app::{App, Plugin, Startup, Update},
    ecs::{
        message::MessageReader,
        system::{Commands, Local, NonSendMarker, Res, ResMut, Single},
    },
    math::Vec2,
    prelude::{ButtonInput, Entity, IntoScheduleConfigs, MouseButton, Resource, SystemSet},
    time::{Time, Timer, TimerMode},
    window::{Window, WindowMoved, WindowPosition, WindowResized},
    winit::WINIT_WINDOWS,
};
use bevy_ui_render::prelude::UiMaterialPlugin;

use crate::{
    config::LocalConfig,
    mask::{
        mapping::cursor::CursorFrameSet,
        mask_command::{
            MaskSize, PendingWindowFocus, TitlebarState, apply_pending_window_focus,
            handle_mask_command, physical_to_logical_i32,
        },
        ui::basic::TITLEBAR_HEIGHT,
        video::{YuvVideoMaterial, handle_video_msg},
    },
    utils::{ChannelSenderWS, DeviceOrientation, share::ControlledDevice},
    web::ws::WebSocketNotification,
};

#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MaskFrameSet {
    Resize,
}

pub struct MaskPlugins;

impl Plugin for MaskPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<YuvVideoMaterial>::default())
            .add_plugins((ui::UiPlugins, mapping::MappingPlugins))
            .init_resource::<PendingWindowFocus>()
            .init_resource::<MaskResizeState>()
            .configure_sets(
                Update,
                (MaskFrameSet::Resize, CursorFrameSet::UpdatePosition).chain(),
            )
            .add_systems(Startup, (init_mask_size, init_titlebar_state))
            .add_systems(
                Update,
                (
                    sync_mask_size.in_set(MaskFrameSet::Resize),
                    sync_mask_position,
                    handle_mask_command,
                    apply_pending_window_focus.after(handle_mask_command),
                    handle_video_msg,
                ),
            );
    }
}

fn init_mask_size(mut commands: Commands, window: Single<&Window>) {
    let config = LocalConfig::get();
    let mask_h = if config.titlebar_visible {
        (window.size().y - TITLEBAR_HEIGHT).max(0.0)
    } else {
        window.size().y
    };
    commands.insert_resource(MaskSize(Vec2::new(window.size().x, mask_h)));
}

fn init_titlebar_state(mut commands: Commands) {
    let config = LocalConfig::get();
    commands.insert_resource(TitlebarState {
        visible: config.titlebar_visible,
    });
}

const DEBOUNCE_MS: u64 = 200;

#[derive(Resource)]
pub struct MaskResizeState {
    active: bool,
    pending_apply: bool,
    timer: Timer,
}

impl Default for MaskResizeState {
    fn default() -> Self {
        Self {
            active: false,
            pending_apply: false,
            timer: Timer::new(Duration::from_millis(DEBOUNCE_MS), TimerMode::Once),
        }
    }
}

impl MaskResizeState {
    fn cancel(&mut self) {
        self.active = false;
        self.pending_apply = false;
        self.timer.reset();
    }
    pub fn begin_interaction(&mut self) {
        self.active = true;
        self.timer.reset();
    }

    fn mark_resized(&mut self) {
        self.begin_interaction();
        self.pending_apply = true;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    fn tick(&mut self, delta: Duration, mouse_input: &ButtonInput<MouseButton>) -> bool {
        if !self.active {
            return false;
        }

        if mouse_input.pressed(MouseButton::Left) {
            self.timer.reset();
            return false;
        }

        self.timer.tick(delta);
        if !self.timer.just_finished() {
            return false;
        }

        self.active = false;
        std::mem::take(&mut self.pending_apply)
    }
}

#[derive(Default)]
struct MoveDebounce {
    timer: Timer,
    pending: bool,
}

impl MoveDebounce {
    fn ensure_init(&mut self) {
        if self.timer.duration() == Duration::ZERO {
            self.timer = Timer::new(Duration::from_millis(DEBOUNCE_MS), TimerMode::Once);
        }
    }
}

fn sync_mask_size(
    mut resize_reader: MessageReader<WindowResized>,
    titlebar_state: Res<TitlebarState>,
    mut mask_size: ResMut<MaskSize>,
    mut window: Single<(Entity, &mut Window)>,
    _main_thread: NonSendMarker,
    time: Res<Time>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    mut resize_state: ResMut<MaskResizeState>,
    ws_tx: Res<ChannelSenderWS>,
) {
    let (entity, window) = &mut *window;
    let minimized = WINIT_WINDOWS.with_borrow(|windows| windows.get_window(*entity)
        .and_then(|w| w.is_minimized()).unwrap_or(false));
    if !window_geometry_is_valid(window.size(), window.position, minimized) {
        resize_reader.clear();
        resize_state.cancel();
        return;
    }
    for e in resize_reader.read() {
        let h = (e.height - titlebar_state.offset()).max(0.0);
        if e.width < 200.0 || h < 80.0 { continue; }
        mask_size.0 = Vec2::new(e.width, h);
        resize_state.mark_resized();
    }

    if resize_state.tick(time.delta(), &mouse_input) {
        if let Some(device) = ControlledDevice::get_main_device_blocking() {
            let (dw, dh) = device.device_size;
            if dw == 0 || dh == 0 {
                return;
            }
            let device_w = dw as f32;
            let device_h = dh as f32;
            let orientation = DeviceOrientation::from_size(dw, dh);
            let titlebar_offset = titlebar_state.offset();
            let current_w = mask_size.0.x;
            let current_h = mask_size.0.y;

            match orientation {
                DeviceOrientation::Landscape => {
                    let target_h = (current_w * (device_h / device_w)).round();
                    if target_h != current_h {
                        window.resolution.set(current_w, target_h + titlebar_offset);
                        mask_size.0 = Vec2::new(current_w, target_h);
                    }
                }
                DeviceOrientation::Portrait => {
                    let target_w = (current_h * (device_w / device_h)).round();
                    if target_w != current_w {
                        window.resolution.set(target_w, current_h + titlebar_offset);
                        mask_size.0 = Vec2::new(target_w, current_h);
                    }
                }
            }

            // Persist size and position after debounce settles
            let content_w = mask_size.0.x.round() as u32;
            let content_h = mask_size.0.y.round() as u32;
            let WindowPosition::At(pos) = window.position else {
                return;
            };
            let scale_factor = window.resolution.scale_factor() as f32;
            let content_top = if titlebar_state.visible {
                physical_to_logical_i32(pos.y, scale_factor) + TITLEBAR_HEIGHT.round() as i32
            } else {
                physical_to_logical_i32(pos.y, scale_factor)
            };
            let content_left = physical_to_logical_i32(pos.x, scale_factor);

            match orientation {
                DeviceOrientation::Landscape => {
                    LocalConfig::set_horizontal_mask_width(content_w);
                    LocalConfig::set_horizontal_position((content_left, content_top));
                    let _ = ws_tx.0.send(WebSocketNotification::ConfigChanged {
                        keys: vec!["horizontal_mask_width".into(), "horizontal_position".into()],
                    });
                }
                DeviceOrientation::Portrait => {
                    LocalConfig::set_vertical_mask_height(content_h);
                    LocalConfig::set_vertical_position((content_left, content_top));
                    let _ = ws_tx.0.send(WebSocketNotification::ConfigChanged {
                        keys: vec!["vertical_mask_height".into(), "vertical_position".into()],
                    });
                }
            }
        }
    }
}

fn sync_mask_position(
    mut move_reader: MessageReader<WindowMoved>,
    window: Single<(Entity, &Window)>,
    _main_thread: NonSendMarker,
    titlebar_state: Res<TitlebarState>,
    time: Res<Time>,
    mut debounce: Local<MoveDebounce>,
    ws_tx: Res<ChannelSenderWS>,
) {
    debounce.ensure_init();
    let (entity, window) = *window;
    let minimized = WINIT_WINDOWS.with_borrow(|windows| windows.get_window(entity)
        .and_then(|w| w.is_minimized()).unwrap_or(false));
    if !window_geometry_is_valid(window.size(), window.position, minimized) {
        move_reader.clear();
        debounce.pending = false;
        return;
    }

    for _ in move_reader.read() {
        debounce.timer.reset();
        debounce.pending = true;
    }

    if debounce.pending {
        debounce.timer.tick(time.delta());
        if debounce.timer.just_finished() {
            debounce.pending = false;
            if let Some(device) = ControlledDevice::get_main_device_blocking() {
                let (dw, dh) = device.device_size;
                if dw == 0 || dh == 0 {
                    return;
                }
                let WindowPosition::At(pos) = window.position else {
                    return;
                };
                let scale_factor = window.resolution.scale_factor() as f32;
                let content_top = if titlebar_state.visible {
                    physical_to_logical_i32(pos.y, scale_factor) + TITLEBAR_HEIGHT.round() as i32
                } else {
                    physical_to_logical_i32(pos.y, scale_factor)
                };
                let content_left = physical_to_logical_i32(pos.x, scale_factor);

                match DeviceOrientation::from_size(dw, dh) {
                    DeviceOrientation::Landscape => {
                        LocalConfig::set_horizontal_position((content_left, content_top));
                        let _ = ws_tx.0.send(WebSocketNotification::ConfigChanged {
                            keys: vec!["horizontal_position".into()],
                        });
                    }
                    DeviceOrientation::Portrait => {
                        LocalConfig::set_vertical_position((content_left, content_top));
                        let _ = ws_tx.0.send(WebSocketNotification::ConfigChanged {
                            keys: vec!["vertical_position".into()],
                        });
                    }
                }
            }
        }
    }
}

fn window_geometry_is_valid(size: Vec2, position: WindowPosition, minimized: bool) -> bool {
    if minimized || !size.is_finite() || size.x < 200.0 || size.y < 80.0 {
        return false;
    }
    // Reject Windows' minimized coordinates, including values converted by DPI scaling.
    !matches!(position, WindowPosition::At(pos) if pos.x <= -10000 || pos.y <= -10000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_systems_run_without_legacy_winit_resource_and_ignore_minimize_events() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(TitlebarState { visible: true })
            .insert_resource(MaskSize(Vec2::new(1000.0, 700.0)))
            .insert_resource(MaskResizeState::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(ChannelSenderWS(tokio::sync::broadcast::channel(8).0))
            .add_message::<WindowResized>()
            .add_message::<WindowMoved>()
            .add_systems(Update, (sync_mask_size, sync_mask_position));
        let entity = app.world_mut().spawn(Window {
            position: WindowPosition::At((-32000, -32000).into()),
            ..Default::default()
        }).id();
        app.world_mut().write_message(WindowResized { window: entity, width: 158.0, height: 28.0 });
        app.world_mut().write_message(WindowMoved { window: entity, position: (-32000, -32000).into() });
        app.update();
        assert_eq!(app.world().resource::<MaskSize>().0, Vec2::new(1000.0, 700.0));
        assert!(!app.world().resource::<MaskResizeState>().active());
    }

    #[test]
    fn minimized_geometry_never_replaces_saved_window_geometry() {
        for pos in [(-32000, -32000), (-21333, -21303)] {
            assert!(!window_geometry_is_valid(Vec2::new(158.0, 28.0), WindowPosition::At(pos.into()), false));
            assert!(!window_geometry_is_valid(Vec2::new(1000.0, 720.0), WindowPosition::At(pos.into()), false));
        }
        assert!(!window_geometry_is_valid(Vec2::new(1000.0, 720.0), WindowPosition::At((100, 100).into()), true));
        assert!(window_geometry_is_valid(Vec2::new(1000.0, 720.0), WindowPosition::At((-1920, 100).into()), false));
    }
}
