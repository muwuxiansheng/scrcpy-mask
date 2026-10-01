use crate::{
    mask::{
        mapping::{
            cursor::{
                ActiveCursorFpsConfig, CursorPosition, IgnoreFirstMotion, release_fps_touches,
                restore_fps_touch,
            },
            utils::{ControlMsgHelper, Position},
            binding::{ButtonBinding, ValidateMappingConfig},
            config::{ActiveMappingConfig, BindMappingType},
        },
        mask_command::MaskSize,
    },
    scrcpy::constant::MotionEventAction,
    utils::{ChannelSenderCS, share::ControlledDevice},
};
use bevy::{input::{ButtonState, mouse::{AccumulatedMouseMotion, MouseButtonInput}}, prelude::*};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

const TOUCH_ID: u64 = 0x7fff_fffe;

fn phone_start_position(normalized: Vec2, phone_size: Vec2) -> Vec2 {
    (phone_size * normalized).clamp(Vec2::ZERO, phone_size - Vec2::ONE)
}

use bevy_ineffable::prelude::{Ineffable, InputBinding, PulseBinding};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MappingDevicePointer {
    #[serde(default = "crate::mask::mapping::config::default_mapping_id")]
    pub id: String,
    pub note: String,
    pub position: Position,
    pub bind: ButtonBinding,
}
#[derive(Debug, Clone)]
pub struct BindMappingDevicePointer {
    pub position: Position,
    pub bind: ButtonBinding,
    pub input_binding: InputBinding,
}
impl From<MappingDevicePointer> for BindMappingDevicePointer {
    fn from(value: MappingDevicePointer) -> Self {
        Self {
            position: value.position,
            input_binding: PulseBinding::just_pressed(value.bind.clone()).0,
            bind: value.bind,
        }
    }
}
impl ValidateMappingConfig for MappingDevicePointer {
    fn validate(&self) -> Result<(), String> {
        if self.position.x < 0 || self.position.y < 0 {
            return Err("Phone pointer start position must be nonnegative".into());
        }
        Ok(())
    }
}
fn requested_start(ineffable: &Ineffable, mappings: &ActiveMappingConfig) -> Option<Vec2> {
    let config = mappings.0.as_ref()?;
    for (action, mapping) in &config.mappings {
        if let BindMappingType::DevicePointer(mapping) = mapping {
            if ineffable.just_pulsed(action.ineff_pulse()) {
                return Some(Vec2::from(mapping.position) / Vec2::from(config.original_size));
            }
        }
    }
    None
}

#[derive(Resource, Default)]
pub struct DevicePointer {
    pub active: bool,
    position: Vec2,
    size: Vec2,
    clicking: bool,
    click_started: Option<Instant>,
    touch_position: Vec2,
    button_events: VecDeque<(ButtonState, Vec2)>,
    connection: Option<BufReader<TcpStream>>,
}

impl DevicePointer {
    fn command(&mut self, command: &str) -> std::io::Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| std::io::Error::other("pointer service not connected"))?;
        connection.get_mut().write_all(command.as_bytes())?;
        let mut response = String::new();
        connection.read_line(&mut response)?;
        if response.trim() == "OK" {
            Ok(())
        } else {
            Err(std::io::Error::other(response))
        }
    }
    fn show(&mut self) -> std::io::Result<()> {
        self.command(&format!(
            "SHOW {} {}\n",
            self.position.x as i32, self.position.y as i32
        ))
    }
    fn start(&mut self, normalized_start: Vec2) -> std::io::Result<()> {
        let device = ControlledDevice::get_main_device_blocking()
            .ok_or_else(|| std::io::Error::other("phone not connected"))?;
        self.size = Vec2::new(device.device_size.0 as f32, device.device_size.1 as f32);
        if self.size.min_element() <= 0.0 {
            return Err(std::io::Error::other("phone size unknown"));
        }
        let address = SocketAddr::from(([127, 0, 0, 1], 27821));
        let socket = TcpStream::connect_timeout(&address, Duration::from_millis(300))?;
        socket.set_nodelay(true)?;
        socket.set_read_timeout(Some(Duration::from_millis(300)))?;
        socket.set_write_timeout(Some(Duration::from_millis(300)))?;
        self.connection = Some(BufReader::new(socket));
        self.position = phone_start_position(normalized_start, self.size);
        self.show()?;
        self.active = true;
        Ok(())
    }
    fn release_click(&mut self, sender: &ChannelSenderCS) {
        if self.clicking {
            ControlMsgHelper::send_touch(
                &sender.0,
                MotionEventAction::Up,
                TOUCH_ID,
                self.size,
                self.touch_position,
            );
            self.clicking = false;
        }
        self.click_started = None;
        self.button_events.clear();
    }
    fn stop(&mut self, sender: &ChannelSenderCS) {
        self.release_click(sender);
        if self.connection.is_some() {
            let _ = self.command("HIDE\n");
        }
        self.connection = None;
        self.active = false;
    }
}

pub fn pointer_active(pointer: Res<DevicePointer>) -> bool {
    pointer.active
}
pub fn toggle_requested(
    ineffable: Res<Ineffable>,
    mappings: Res<ActiveMappingConfig>,
    window: Single<&Window>,
) -> bool {
    window.focused && requested_start(&ineffable, &mappings).is_some()
}

pub(super) fn toggle_pointer(
    ineffable: Res<Ineffable>,
    mappings: Res<ActiveMappingConfig>,
    mut pointer: ResMut<DevicePointer>,
    mut fps: ResMut<ActiveCursorFpsConfig>,
    mut position: ResMut<CursorPosition>,
    mask: Res<MaskSize>,
    sender: Res<ChannelSenderCS>,
    mut ignore: ResMut<IgnoreFirstMotion>,
) {
    if pointer.active {
        pointer.stop(&sender);
        fps.ignore_fps_motion = false;
        restore_fps_touch(&sender.0, &mut fps);
        position.0 = fps.original_pos / fps.original_size * mask.0;
        log::info!("[DevicePointer] returned to FPS view");
    } else {
        release_fps_touches(&sender.0, &mut fps, mask.0, position.0);
        let Some(start) = requested_start(&ineffable, &mappings) else { return; };
        if let Err(error) = pointer.start(start) {
            pointer.stop(&sender);
            restore_fps_touch(&sender.0, &mut fps);
            position.0 = fps.original_pos / fps.original_size * mask.0;
            log::error!("[DevicePointer] cannot show phone pointer: {error}");
        } else {
            log::info!("[DevicePointer] phone pointer enabled; left button uses touch");
        }
    }
    ignore.0 = true;
}

pub(super) fn handle_pointer_motion(
    mut pointer: ResMut<DevicePointer>,
    motion: Res<AccumulatedMouseMotion>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut button_input: MessageReader<MouseButtonInput>,
    window: Single<&Window>,
    mask: Res<MaskSize>,
    sender: Res<ChannelSenderCS>,
    mut ignore: ResMut<IgnoreFirstMotion>,
    mut fps: ResMut<ActiveCursorFpsConfig>,
    mut fps_position: ResMut<CursorPosition>,
) {
    let events: Vec<_> = button_input.read().filter(|e| e.button == MouseButton::Left).map(|e| e.state).collect();
    if !pointer.active { return; }
    if !window.focused {
        pointer.release_click(&sender);
        return;
    }
    // Ignore a capture warp, but always process button transitions on that frame.
    let delta = if ignore.0 { ignore.0 = false; Vec2::ZERO } else { motion.delta };
    if delta != Vec2::ZERO {
        let size = pointer.size;
        pointer.position =
            (pointer.position + delta / mask.0 * size).clamp(Vec2::ZERO, size - Vec2::ONE);
        if let Err(error) = pointer.show() {
            log::error!("[DevicePointer] service connection lost: {error}");
            pointer.stop(&sender);
            restore_fps_touch(&sender.0, &mut fps);
            fps_position.0 = fps.original_pos / fps.original_size * mask.0;
            return;
        }
    }
    let pos = pointer.position;
    if events.is_empty() {
        // Also support synthesized ButtonInput.
        if mouse.just_pressed(MouseButton::Left) {
            pointer.button_events.push_back((ButtonState::Pressed, pos));
        }
        if mouse.just_released(MouseButton::Left) || (pointer.clicking && !mouse.pressed(MouseButton::Left) && pointer.button_events.is_empty()) {
            pointer.button_events.push_back((ButtonState::Released, pos));
        }
    } else {
        for state in events { pointer.button_events.push_back((state, pos)); }
    }
    while let Some(&(state, event_pos)) = pointer.button_events.front() {
        if state == ButtonState::Released && pointer.clicking && pointer.click_started.is_some_and(|t| t.elapsed() < Duration::from_millis(50)) {
            break;
        }
        pointer.button_events.pop_front();
        match state {
            ButtonState::Pressed if !pointer.clicking => {
                pointer.clicking = true;
                pointer.click_started = Some(Instant::now());
                pointer.touch_position = event_pos;
                ControlMsgHelper::send_touch(&sender.0, MotionEventAction::Down, TOUCH_ID, pointer.size, event_pos);
            }
            ButtonState::Released if pointer.clicking => {
                pointer.clicking = false;
                pointer.click_started = None;
                pointer.touch_position = event_pos;
                ControlMsgHelper::send_touch(&sender.0, MotionEventAction::Up, TOUCH_ID, pointer.size, event_pos);
            }
            _ => {}
        }
    }
    if pointer.clicking && pointer.button_events.is_empty() && delta != Vec2::ZERO {
        pointer.touch_position = pos;
        ControlMsgHelper::send_touch(&sender.0, MotionEventAction::Move, TOUCH_ID, pointer.size, pos);
    }
}

pub fn cleanup_pointer(mut pointer: ResMut<DevicePointer>, sender: Res<ChannelSenderCS>) {
    pointer.stop(&sender);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;
    #[test]
    fn configurable_pointer_survives_save_and_load_with_keyboard_binding() {
        use crate::mask::mapping::config::{MappingConfig, BindMappingConfig, MappingType, MappingAction};
        let config: MappingConfig = serde_json::from_value(serde_json::json!({
            "version": "0.9.0", "original_size": {"width": 1280, "height": 906},
            "mappings": [{"type": "DevicePointer", "id": "pointer-test", "note": "map pointer",
                "bind": ["Tab"], "position": {"x": 320, "y": 453}}]
        })).unwrap();
        let saved = serde_json::to_string(&config).unwrap();
        let reloaded: MappingConfig = serde_json::from_str(&saved).unwrap();
        assert!(matches!(&reloaded.mappings[0], MappingType::DevicePointer(_)));
        let bound = BindMappingConfig::from(reloaded);
        let mapping = bound.mappings.get(&MappingAction::DevicePointer1).unwrap().as_ref_devicepointer();
        assert_eq!(mapping.bind.to_string(), "Tab");
        assert_eq!(phone_start_position(Vec2::from(mapping.position) / Vec2::from(bound.original_size), Vec2::new(3392.0, 2400.0)), Vec2::new(848.0, 1200.0));
    }
    #[test]
    fn configured_pointer_start_is_clamped_inside_phone_screen() {
        assert_eq!(phone_start_position(Vec2::new(1.2, -0.2), Vec2::new(3392.0, 2400.0)), Vec2::new(3391.0, 0.0));
    }
    fn test_app() -> (
        App,
        broadcast::Receiver<crate::scrcpy::control_msg::ScrcpyControlMsg>,
    ) {
        let (tx, rx) = broadcast::channel(16);
        let mut app = App::new();
        app.insert_resource(DevicePointer {
            active: true,
            position: Vec2::new(123.0, 456.0),
            size: Vec2::new(2000.0, 1000.0),
            ..default()
        })
        .insert_resource(ChannelSenderCS(tx))
        .insert_resource(ActiveCursorFpsConfig::default())
        .insert_resource(CursorPosition(Vec2::ZERO))
        .insert_resource(MaskSize(Vec2::new(1000.0, 500.0)))
        .insert_resource(IgnoreFirstMotion(false))
        .insert_resource(AccumulatedMouseMotion::default())
        .init_resource::<Messages<MouseButtonInput>>()
        .insert_resource(ButtonInput::<MouseButton>::default());
        app.world_mut().spawn(Window {
            focused: true,
            ..default()
        });
        (app, rx)
    }
    #[test]
    fn dragging_updates_the_overlay_and_touch_to_the_same_coordinates() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            reader.get_mut().write_all(b"OK\n").unwrap();
            command
        });
        let (mut app, mut rx) = test_app();
        {
            let mut pointer = app.world_mut().resource_mut::<DevicePointer>();
            pointer.connection = Some(BufReader::new(socket));
            pointer.clicking = true;
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(100.0, 25.0);
        app.add_systems(Update, handle_pointer_motion);
        app.update();
        assert_eq!(server.join().unwrap(), "SHOW 323 506\n");
        assert!(matches!(
            rx.try_recv().unwrap(),
            crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent {
                action: MotionEventAction::Move,
                x: 323,
                y: 506,
                ..
            }
        ));
    }

    #[test]
    fn left_button_uses_touch_at_the_phone_pointer_position() {
        let (mut app, mut rx) = test_app();
        app.add_systems(Update, handle_pointer_motion);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        let down = rx.try_recv().unwrap();
        assert!(matches!(
            down,
            crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent {
                action: MotionEventAction::Down,
                pointer_id: TOUCH_ID,
                x: 123,
                y: 456,
                w: 2000,
                h: 1000,
                ..
            }
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.world_mut().resource_mut::<DevicePointer>().click_started = Some(Instant::now() - Duration::from_millis(60));
        app.update();
        assert!(matches!(
            rx.try_recv().unwrap(),
            crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent {
                action: MotionEventAction::Up,
                pointer_id: TOUCH_ID,
                ..
            }
        ));
        assert!(!app.world().resource::<DevicePointer>().clicking);
    }
    #[test]
    fn focus_loss_releases_a_held_pointer_touch() {
        let (mut app, mut rx) = test_app();
        app.world_mut().resource_mut::<DevicePointer>().clicking = true;
        let mut query = app.world_mut().query::<&mut Window>();
        query.single_mut(app.world_mut()).unwrap().focused = false;
        app.add_systems(Update, handle_pointer_motion);
        app.update();
        assert!(matches!(
            rx.try_recv().unwrap(),
            crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent {
                action: MotionEventAction::Up,
                ..
            }
        ));
        assert!(!app.world().resource::<DevicePointer>().clicking);
    }
    #[test]
    fn master_switch_cleanup_releases_touch_and_exits_pointer_mode() {
        let (mut app, mut rx) = test_app();
        app.world_mut().resource_mut::<DevicePointer>().clicking = true;
        app.add_systems(Update, cleanup_pointer);
        app.update();
        assert!(!app.world().resource::<DevicePointer>().active);
        assert!(matches!(
            rx.try_recv().unwrap(),
            crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent {
                action: MotionEventAction::Up,
                ..
            }
        ));
    }

    fn queue_left(app: &mut App, state: ButtonState) {
        let entity = app.world_mut().query_filtered::<Entity, With<Window>>().single(app.world()).unwrap();
        app.world_mut().resource_mut::<Messages<MouseButtonInput>>().write(MouseButtonInput {
            button: MouseButton::Left, state, window: entity,
        });
    }
    #[test]
    fn first_click_after_pointer_capture_is_not_discarded() {
        let (mut app, mut rx) = test_app();
        app.world_mut().resource_mut::<IgnoreFirstMotion>().0 = true;
        queue_left(&mut app, ButtonState::Pressed);
        app.add_systems(Update, handle_pointer_motion);
        app.update();
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Down, x: 123, y: 456, .. }));
    }
    #[test]
    fn rapid_click_preserves_down_and_delays_up_even_without_mouse_motion() {
        let (mut app, mut rx) = test_app();
        queue_left(&mut app, ButtonState::Pressed);
        queue_left(&mut app, ButtonState::Released);
        app.add_systems(Update, handle_pointer_motion);
        app.update();
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Down, .. }));
        assert!(rx.try_recv().is_err());
        app.world_mut().resource_mut::<DevicePointer>().click_started = Some(Instant::now() - Duration::from_millis(60));
        app.update();
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Up, .. }));
        assert!(!app.world().resource::<DevicePointer>().clicking);
    }
    #[test]
    fn rapid_double_click_keeps_both_gestures() {
        let (mut app, mut rx) = test_app();
        for _ in 0..2 {
            queue_left(&mut app, ButtonState::Pressed);
            queue_left(&mut app, ButtonState::Released);
        }
        app.add_systems(Update, handle_pointer_motion);
        app.update();
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Down, .. }));
        app.world_mut().resource_mut::<DevicePointer>().click_started = Some(Instant::now() - Duration::from_millis(60));
        app.update();
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Up, .. }));
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Down, .. }));
        app.world_mut().resource_mut::<DevicePointer>().click_started = Some(Instant::now() - Duration::from_millis(60));
        app.update();
        assert!(matches!(rx.try_recv().unwrap(), crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent { action: MotionEventAction::Up, .. }));
    }
}
