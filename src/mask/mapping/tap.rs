use std::{collections::HashMap, time::Duration};

use crate::tokio_tasks::TokioTasksRuntime;
use bevy::{
    ecs::{
        resource::Resource,
        system::{Commands, Res, ResMut},
    },
    math::Vec2,
    state::state::State,
    time::{Time, Timer, TimerMode},
};
use bevy_ineffable::prelude::{ContinuousBinding, Ineffable, InputBinding, PulseBinding};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::{
    mask::mapping::{
        MappingState,
        binding::{ButtonBinding, ValidateMappingConfig},
        config::ActiveMappingConfig,
        cursor::{CursorPosition, CursorState},
        executor::{
            MappingExecutionError, MappingLifecycleStart, MappingLifecycleState,
            make_mapping_execution_context, run_script_hook, run_with_hooks,
        },
        script::{BindMappingScriptHooks, MappingScriptHooks},
        script_helper::{ScriptRuntimeCommandSender, ScriptSharedState},
        utils::{ControlMsgHelper, Position, default_random_offset, random_offset_vec2},
    },
    mask::mask_command::MaskSize,
    scrcpy::constant::MotionEventAction,
    utils::ChannelSenderCS,
};

pub fn tap_init(mut commands: Commands) {
    commands.insert_resource(ActiveRepeatTapMap::default());
    commands.insert_resource(ActiveSingleTapMap::default());
    commands.insert_resource(SingleTapLifecycleState::default());
    commands.insert_resource(RepeatTapLifecycleState::default());
}

pub fn cleanup_tap_on_stop(
    active_mapping: Res<ActiveMappingConfig>,
    cs_tx_res: Res<ChannelSenderCS>,
    mut active_single_tap: ResMut<ActiveSingleTapMap>,
    mut active_repeat_tap: ResMut<ActiveRepeatTapMap>,
    mut single_lifecycle_state: ResMut<SingleTapLifecycleState>,
    mut repeat_lifecycle_state: ResMut<RepeatTapLifecycleState>,
) {
    if let Some(active_mapping) = &active_mapping.0 {
        let original_size: Vec2 = active_mapping.original_size.into();
        for (_, touch) in active_single_tap.0.drain() {
            ControlMsgHelper::send_touch(
                &cs_tx_res.0,
                MotionEventAction::Up,
                touch.pointer_id,
                original_size,
                touch.position,
            );
        }
    } else {
        active_single_tap.0.clear();
    }

    active_repeat_tap.0.clear();
    single_lifecycle_state.0.clear_all();
    repeat_lifecycle_state.0.clear_all();
}

#[derive(Debug, Clone)]
pub struct BindMappingSingleTap {
    pub id: String,
    pub position: Position,
    pub note: String,
    pub pointer_id: u64,
    pub duration: u64,
    pub sync: bool,
    pub bind: ButtonBinding,
    pub input_binding: InputBinding,
    pub random_offset_x: f32,
    pub random_offset_y: f32,
    pub hold_jitter_enabled: bool,
    pub hold_jitter_x: f32,
    pub hold_jitter_y: f32,
    pub hold_jitter_interval_ms: u64,
    pub script_hooks: BindMappingScriptHooks,
}

impl From<MappingSingleTap> for BindMappingSingleTap {
    fn from(value: MappingSingleTap) -> Self {
        Self {
            id: value.id,
            position: value.position,
            note: value.note,
            pointer_id: value.pointer_id,
            duration: value.duration,
            sync: value.sync,
            bind: value.bind.clone(),
            random_offset_x: value.random_offset_x,
            random_offset_y: value.random_offset_y,
            hold_jitter_enabled: value.hold_jitter_enabled,
            hold_jitter_x: value.hold_jitter_x,
            hold_jitter_y: value.hold_jitter_y,
            hold_jitter_interval_ms: value.hold_jitter_interval_ms,
            script_hooks: value.script_hooks.into(),
            input_binding: ContinuousBinding::hold(value.bind).0,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MappingSingleTap {
    #[serde(default = "crate::mask::mapping::config::default_mapping_id")]
    pub id: String,
    pub position: Position,
    pub note: String,
    pub pointer_id: u64,
    pub duration: u64,
    pub sync: bool,
    pub bind: ButtonBinding,
    #[serde(
        default = "default_random_offset",
        serialize_with = "crate::mask::mapping::serde_float::serialize_f32_3dp"
    )]
    pub random_offset_x: f32,
    #[serde(
        default = "default_random_offset",
        serialize_with = "crate::mask::mapping::serde_float::serialize_f32_3dp"
    )]
    pub random_offset_y: f32,
    #[serde(default)]
    pub hold_jitter_enabled: bool,
    #[serde(default = "default_hold_jitter")]
    pub hold_jitter_x: f32,
    #[serde(default = "default_hold_jitter")]
    pub hold_jitter_y: f32,
    #[serde(default = "default_hold_jitter_interval")]
    pub hold_jitter_interval_ms: u64,
    #[serde(default)]
    pub script_hooks: MappingScriptHooks,
}

impl ValidateMappingConfig for MappingSingleTap {
    fn validate(&self) -> Result<(), String> {
        self.validate_hold_jitter()?;
        self.script_hooks.validate()
    }
}

#[derive(Resource, Default)]
pub struct ActiveSingleTapMap(HashMap<String, HeldTapTouch>);

fn default_hold_jitter() -> f32 { 2.0 }
fn default_hold_jitter_interval() -> u64 { 160 }

#[cfg(test)]
mod hold_jitter_tests {
    use super::*;

    #[test]
    fn hold_jitter_is_delayed_bounded_and_releases_at_last_move() {
        let mut touch = HeldTapTouch::new(Vec2::new(100.0, 100.0), 3);
        let range = Vec2::splat(2.0);
        for _ in 0..15 {
            assert!(touch.advance(0.016, range, 0.16, Vec2::splat(200.0)).is_none());
        }
        assert_eq!(touch.position, touch.anchor);
        let mut moves = 0;
        for _ in 0..600 {
            if let Some(pos) = touch.advance(0.016, range, 0.16, Vec2::splat(200.0)) {
                moves += 1;
                assert!((pos - touch.anchor).abs().max_element() <= 2.001);
            }
        }
        assert!(moves > 0);
        let last = touch.position;
        let mapping: MappingSingleTap = serde_json::from_value(serde_json::json!({
            "id":"test", "position":{"x":100,"y":100}, "note":"", "pointer_id":3,
            "duration":50, "sync":true, "bind":["KeyF"]
        })).unwrap();
        assert!(!mapping.hold_jitter_enabled);
        let mapping = mapping.into();
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        let mut active = ActiveSingleTapMap::default();
        active.0.insert("test".into(), touch);
        assert!(apply_single_tap_up(&ChannelSenderCS(tx), &mut active, "test", &mapping, Vec2::splat(200.0)));
        match rx.try_recv().unwrap() {
            crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent {action,x,y,..} => {
                assert_eq!(action, MotionEventAction::Up);
                assert_eq!((x,y), (last.x as i32,last.y as i32));
            }
            _ => panic!("Expected touch up"),
        }
        assert!(active.0.is_empty());
    }

    #[test]
    fn zero_hold_jitter_never_moves_even_during_long_hold() {
        let mut touch = HeldTapTouch::new(Vec2::splat(100.0), 3);
        for _ in 0..100 { assert!(touch.advance(0.1, Vec2::ZERO, 0.16, Vec2::splat(200.0)).is_none()); }
        assert_eq!(touch.position, touch.anchor);
    }
}

impl MappingSingleTap {
    pub fn validate_hold_jitter(&self) -> Result<(), String> {
        if !self.hold_jitter_x.is_finite() || !self.hold_jitter_y.is_finite()
            || self.hold_jitter_x < 0.0 || self.hold_jitter_y < 0.0 {
            return Err("长按微动 X/Y 范围必须是非负有限数值".into());
        }
        if self.hold_jitter_interval_ms < 50 || self.hold_jitter_interval_ms > 5000 {
            return Err("长按微动周期必须在 50～5000ms 之间".into());
        }
        Ok(())
    }
}

struct HeldTapTouch {
    position: Vec2,
    anchor: Vec2,
    from: Vec2,
    target: Vec2,
    elapsed: f32,
    phase: f32,
    send_elapsed: f32,
    started: bool,
    pointer_id: u64,
}

impl HeldTapTouch {
    fn new(position: Vec2, pointer_id: u64) -> Self {
        Self { position, anchor: position, from: position, target: position,
            elapsed: 0.0, phase: 0.0, send_elapsed: 0.0, started: false, pointer_id }
    }

    fn advance(&mut self, dt: f32, range: Vec2, period: f32, screen: Vec2) -> Option<Vec2> {
        let before = self.elapsed;
        self.elapsed += dt;
        // Short taps keep their original touch location; movement begins after 250ms.
        if self.elapsed <= 0.25 || range == Vec2::ZERO { return None; }
        let active_dt = self.elapsed - before.max(0.25);
        if !self.started || self.phase >= period {
            self.from = self.position;
            self.target = random_offset_vec2(self.anchor, range).clamp(Vec2::ZERO, screen - Vec2::ONE);
            self.phase = 0.0;
            self.started = true;
        }
        self.phase += active_dt;
        self.send_elapsed += active_dt;
        if self.send_elapsed < 0.016 { return None; }
        self.send_elapsed %= 0.016;
        let t = (self.phase / period).clamp(0.0, 1.0);
        let next = self.from.lerp(self.target, t * t * (3.0 - 2.0 * t));
        if next.as_ivec2() == self.position.as_ivec2() { return None; }
        self.position = next;
        Some(next)
    }
}

#[derive(Resource, Default)]
pub struct SingleTapLifecycleState(MappingLifecycleState<SingleTapReleaseContext>);

#[derive(Clone)]
struct SingleTapReleaseContext {
    cursor_pos: Vec2,
    mask_size: Vec2,
    raw_input_flag: bool,
    fps_mode_flag: bool,
}

fn single_tap_has_before_hook(mapping: &BindMappingSingleTap) -> bool {
    !mapping.script_hooks.before_script_ast.empty
}

fn single_tap_has_after_hook(mapping: &BindMappingSingleTap) -> bool {
    !mapping.script_hooks.after_script_ast.empty
}

fn apply_single_tap_down(
    cs_tx: &ChannelSenderCS,
    active_single_tap: &mut ActiveSingleTapMap,
    action: String,
    mapping: &BindMappingSingleTap,
    original_size: Vec2,
) {
    let random_pos = random_offset_vec2(
        mapping.position.into(),
        Vec2::new(mapping.random_offset_x, mapping.random_offset_y),
    );
    ControlMsgHelper::send_touch(
        &cs_tx.0,
        MotionEventAction::Down,
        mapping.pointer_id,
        original_size,
        random_pos,
    );
    active_single_tap.0.insert(action, HeldTapTouch::new(random_pos, mapping.pointer_id));
}

fn apply_single_tap_up(
    cs_tx: &ChannelSenderCS,
    active_single_tap: &mut ActiveSingleTapMap,
    action: &str,
    mapping: &BindMappingSingleTap,
    original_size: Vec2,
) -> bool {
    if let Some(touch) = active_single_tap.0.remove(action) {
        ControlMsgHelper::send_touch(
            &cs_tx.0,
            MotionEventAction::Up,
            mapping.pointer_id,
            original_size,
            touch.position,
        );
        true
    } else {
        false
    }
}

pub fn handle_single_tap(
    ineffable: Res<Ineffable>,
    pointer: Res<super::device_pointer::DevicePointer>,
    active_mapping: Res<ActiveMappingConfig>,
    mut active_single_tap: ResMut<ActiveSingleTapMap>,
    cs_tx_res: Res<ChannelSenderCS>,
    script_command_tx: Res<ScriptRuntimeCommandSender>,
    shared_state: Res<ScriptSharedState>,
    mask_size: Res<MaskSize>,
    cursor_pos: Res<CursorPosition>,
    mapping_state: Res<State<MappingState>>,
    cursor_state: Res<State<CursorState>>,
    runtime: ResMut<TokioTasksRuntime>,
    mut lifecycle_state: ResMut<SingleTapLifecycleState>,
    time: Res<Time>,
) {
    if let Some(active_mapping) = &active_mapping.0 {
        for (action, mapping) in &active_mapping.mappings {
            if action.as_ref().starts_with("SingleTap") {
                let original_size: Vec2 = active_mapping.original_size.into();
                let mapping = mapping.as_ref_singletap();
                if pointer.active && mapping.bind.has_mouse_binding() {
                    continue;
                }
                if ineffable.just_activated(action.ineff_continuous()) {
                    if mapping.sync {
                        if single_tap_has_before_hook(mapping) {
                            let action = action.to_string();
                            let version = lifecycle_state.0.begin_start(&action);
                            let mapping = mapping.clone();
                            let before_script_ast = mapping.script_hooks.before_script_ast.clone();
                            let after_script_ast = mapping.script_hooks.after_script_ast.clone();
                            let exec_ctx = make_mapping_execution_context(
                                &cs_tx_res,
                                &script_command_tx,
                                &shared_state,
                                mapping.id.clone(),
                                original_size,
                                cursor_pos.0,
                                mask_size.0,
                                mapping_state.get() == &MappingState::RawInput,
                                cursor_state.get() == &CursorState::Fps,
                            );
                            let cs_tx = cs_tx_res.0.clone();
                            runtime.spawn_background_task(move |mut task_ctx| async move {
                                if let Err(e) = run_script_hook(&before_script_ast, &exec_ctx).await
                                {
                                    task_ctx
                                        .run_on_main_thread({
                                            let action = action.clone();
                                            move |main_ctx| {
                                                main_ctx
                                                    .world
                                                    .resource_mut::<SingleTapLifecycleState>()
                                                    .0
                                                    .cancel_start(&action, version);
                                            }
                                        })
                                        .await;
                                    log::error!("[SingleTap] script hook runtime error: {:?}", e);
                                    return;
                                }

                                let pending_release = task_ctx
                                    .run_on_main_thread(move |main_ctx| {
                                        let start = main_ctx
                                            .world
                                            .resource_mut::<SingleTapLifecycleState>()
                                            .0
                                            .finish_start(&action, version);
                                        let pending_release = match start {
                                            MappingLifecycleStart::Stale => return None,
                                            MappingLifecycleStart::Ready { pending_release } => {
                                                pending_release
                                            }
                                        };

                                        let mut active_single_tap =
                                            main_ctx.world.resource_mut::<ActiveSingleTapMap>();
                                        apply_single_tap_down(
                                            &ChannelSenderCS(cs_tx.clone()),
                                            &mut active_single_tap,
                                            action.clone(),
                                            &mapping,
                                            original_size,
                                        );

                                        if pending_release.is_some() {
                                            apply_single_tap_up(
                                                &ChannelSenderCS(cs_tx),
                                                &mut active_single_tap,
                                                &action,
                                                &mapping,
                                                original_size,
                                            );
                                        }

                                        pending_release
                                    })
                                    .await;

                                if let Some(release) = pending_release {
                                    if !after_script_ast.empty {
                                        let mut after_exec_ctx = exec_ctx.clone();
                                        after_exec_ctx.cursor_pos = release.cursor_pos;
                                        after_exec_ctx.mask_size = release.mask_size;
                                        after_exec_ctx.raw_input_flag = release.raw_input_flag;
                                        after_exec_ctx.fps_mode_flag = release.fps_mode_flag;
                                        if let Err(e) =
                                            run_script_hook(&after_script_ast, &after_exec_ctx)
                                                .await
                                        {
                                            log::error!(
                                                "[SingleTap] script hook runtime error: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                            });
                        } else {
                            apply_single_tap_down(
                                &cs_tx_res,
                                &mut active_single_tap,
                                action.to_string(),
                                mapping,
                                original_size,
                            );
                        }
                    } else {
                        let pointer_id = mapping.pointer_id;
                        let random_pos = random_offset_vec2(
                            mapping.position.into(),
                            Vec2::new(mapping.random_offset_x, mapping.random_offset_y),
                        );
                        let duration = Duration::from_millis(mapping.duration as u64);
                        let hooks = mapping.script_hooks.clone();
                        let exec_ctx = make_mapping_execution_context(
                            &cs_tx_res,
                            &script_command_tx,
                            &shared_state,
                            mapping.id.clone(),
                            original_size,
                            cursor_pos.0,
                            mask_size.0,
                            mapping_state.get() == &MappingState::RawInput,
                            cursor_state.get() == &CursorState::Fps,
                        );
                        let generation = exec_ctx.shared_state.generation();
                        runtime.spawn_background_task(move |_ctx| async move {
                            let result = run_with_hooks(hooks, exec_ctx, move |ctx| async move {
                                if ctx.shared_state.generation() != generation { return Ok::<(), MappingExecutionError>(()); }
                                ControlMsgHelper::send_touch(
                                    &ctx.cs_tx,
                                    MotionEventAction::Down,
                                    pointer_id,
                                    ctx.original_size,
                                    random_pos,
                                );
                                sleep(duration).await;
                                if ctx.shared_state.generation() != generation { return Ok::<(), MappingExecutionError>(()); }
                                ControlMsgHelper::send_touch(
                                    &ctx.cs_tx,
                                    MotionEventAction::Up,
                                    pointer_id,
                                    ctx.original_size,
                                    random_pos,
                                );
                                Ok::<(), MappingExecutionError>(())
                            })
                            .await;
                            if let Err(e) = result {
                                log::error!("[SingleTap] mapping execution error: {:?}", e);
                            }
                        });
                    }
                } else if mapping.sync && ineffable.just_deactivated(action.ineff_continuous()) {
                    let released = apply_single_tap_up(
                        &cs_tx_res,
                        &mut active_single_tap,
                        action.as_ref(),
                        mapping,
                        original_size,
                    );

                    if released {
                        lifecycle_state.0.clear_pending(action.as_ref());
                    } else if single_tap_has_before_hook(mapping) {
                        lifecycle_state.0.record_early_release(
                            action.as_ref(),
                            SingleTapReleaseContext {
                                cursor_pos: cursor_pos.0,
                                mask_size: mask_size.0,
                                raw_input_flag: mapping_state.get() == &MappingState::RawInput,
                                fps_mode_flag: cursor_state.get() == &CursorState::Fps,
                            },
                        );
                    }

                    if released && single_tap_has_after_hook(mapping) {
                        let after_script_ast = mapping.script_hooks.after_script_ast.clone();
                        let exec_ctx = make_mapping_execution_context(
                            &cs_tx_res,
                            &script_command_tx,
                            &shared_state,
                            mapping.id.clone(),
                            original_size,
                            cursor_pos.0,
                            mask_size.0,
                            mapping_state.get() == &MappingState::RawInput,
                            cursor_state.get() == &CursorState::Fps,
                        );
                        runtime.spawn_background_task(move |_ctx| async move {
                            if let Err(e) = run_script_hook(&after_script_ast, &exec_ctx).await {
                                log::error!("[SingleTap] script hook runtime error: {:?}", e);
                            }
                        });
                    }
                }
                if mapping.sync && mapping.hold_jitter_enabled {
                    if let Some(touch) = active_single_tap.0.get_mut(action.as_ref()) {
                        if let Some(position) = touch.advance(time.delta_secs(),
                            Vec2::new(mapping.hold_jitter_x, mapping.hold_jitter_y),
                            mapping.hold_jitter_interval_ms as f32 / 1000.0, original_size) {
                            ControlMsgHelper::send_touch(&cs_tx_res.0, MotionEventAction::Move,
                                mapping.pointer_id, original_size, position);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BindMappingRepeatTap {
    pub id: String,
    pub position: Position,
    pub note: String,
    pub pointer_id: u64,
    pub duration: u64,
    pub interval: u32,
    pub bind: ButtonBinding,
    pub input_binding: InputBinding,
    pub random_offset_x: f32,
    pub random_offset_y: f32,
    pub script_hooks: BindMappingScriptHooks,
}

impl From<MappingRepeatTap> for BindMappingRepeatTap {
    fn from(value: MappingRepeatTap) -> Self {
        Self {
            id: value.id,
            position: value.position,
            note: value.note,
            pointer_id: value.pointer_id,
            duration: value.duration,
            interval: value.interval,
            bind: value.bind.clone(),
            input_binding: ContinuousBinding::hold(value.bind).0,
            random_offset_x: value.random_offset_x,
            random_offset_y: value.random_offset_y,
            script_hooks: value.script_hooks.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MappingRepeatTap {
    #[serde(default = "crate::mask::mapping::config::default_mapping_id")]
    pub id: String,
    pub position: Position,
    pub note: String,
    pub pointer_id: u64,
    pub duration: u64,
    pub interval: u32,
    pub bind: ButtonBinding,
    #[serde(
        default = "default_random_offset",
        serialize_with = "crate::mask::mapping::serde_float::serialize_f32_3dp"
    )]
    pub random_offset_x: f32,
    #[serde(
        default = "default_random_offset",
        serialize_with = "crate::mask::mapping::serde_float::serialize_f32_3dp"
    )]
    pub random_offset_y: f32,
    #[serde(default)]
    pub script_hooks: MappingScriptHooks,
}

impl ValidateMappingConfig for MappingRepeatTap {
    fn validate(&self) -> Result<(), String> {
        self.script_hooks.validate()
    }
}

#[derive(Resource, Default)]
pub struct ActiveRepeatTapMap(HashMap<String, RepeatTapTimer>);

#[derive(Resource, Default)]
pub struct RepeatTapLifecycleState(MappingLifecycleState<RepeatTapReleaseContext>);

#[derive(Clone)]
struct RepeatTapReleaseContext {
    cursor_pos: Vec2,
    mask_size: Vec2,
    raw_input_flag: bool,
    fps_mode_flag: bool,
}

struct RepeatTapTimer {
    timer: Timer,
    pointer_id: u64,
    original_pos: Vec2,
    original_size: Vec2,
    duration: Duration,
    random_offset: Vec2,
}

fn repeat_tap_has_before_hook(mapping: &BindMappingRepeatTap) -> bool {
    !mapping.script_hooks.before_script_ast.empty
}

fn repeat_tap_has_after_hook(mapping: &BindMappingRepeatTap) -> bool {
    !mapping.script_hooks.after_script_ast.empty
}

fn spawn_repeat_tap_once(
    runtime: &TokioTasksRuntime,
    cs_tx: &ChannelSenderCS,
    pointer_id: u64,
    original_size: Vec2,
    original_pos: Vec2,
    random_offset: Vec2,
    duration: Duration,
) {
    let cs_tx = cs_tx.0.clone();
    let random_pos = random_offset_vec2(original_pos, random_offset);
    ControlMsgHelper::send_touch(
        &cs_tx,
        MotionEventAction::Down,
        pointer_id,
        original_size,
        random_pos,
    );
    runtime.spawn_background_task(move |_ctx| async move {
        sleep(duration).await;
        ControlMsgHelper::send_touch(
            &cs_tx,
            MotionEventAction::Up,
            pointer_id,
            original_size,
            random_pos,
        );
    });
}

fn make_repeat_tap_timer(mapping: &BindMappingRepeatTap, original_size: Vec2) -> RepeatTapTimer {
    RepeatTapTimer {
        timer: {
            let interval = Duration::from_millis(mapping.interval as u64);
            let mut timer = Timer::new(interval, TimerMode::Repeating);
            timer.tick(interval);
            timer
        },
        pointer_id: mapping.pointer_id,
        original_pos: mapping.position.into(),
        original_size,
        duration: Duration::from_millis(mapping.duration as u64),
        random_offset: Vec2::new(mapping.random_offset_x, mapping.random_offset_y),
    }
}

pub fn handle_repeat_tap_trigger(
    time: Res<Time>,
    mut active_map: ResMut<ActiveRepeatTapMap>,
    cs_tx_res: Res<ChannelSenderCS>,
    runtime: ResMut<TokioTasksRuntime>,
) {
    for (_, timer) in active_map.0.iter_mut() {
        if timer.timer.tick(time.delta()).just_finished() {
            spawn_repeat_tap_once(
                &runtime,
                &cs_tx_res,
                timer.pointer_id,
                timer.original_size,
                timer.original_pos,
                timer.random_offset,
                timer.duration,
            );
        }
    }
}

pub fn handle_repeat_tap(
    ineffable: Res<Ineffable>,
    active_mapping: Res<ActiveMappingConfig>,
    mut active_map: ResMut<ActiveRepeatTapMap>,
    cs_tx_res: Res<ChannelSenderCS>,
    script_command_tx: Res<ScriptRuntimeCommandSender>,
    shared_state: Res<ScriptSharedState>,
    mask_size: Res<MaskSize>,
    cursor_pos: Res<CursorPosition>,
    mapping_state: Res<State<MappingState>>,
    cursor_state: Res<State<CursorState>>,
    runtime: ResMut<TokioTasksRuntime>,
    mut lifecycle_state: ResMut<RepeatTapLifecycleState>,
) {
    if let Some(active_mapping) = &active_mapping.0 {
        for (action, mapping) in &active_mapping.mappings {
            if action.as_ref().starts_with("RepeatTap") {
                let mapping = mapping.as_ref_repeattap();
                if ineffable.just_activated(action.ineff_continuous()) {
                    let original_size: Vec2 = active_mapping.original_size.into();
                    if repeat_tap_has_before_hook(mapping) {
                        let action = action.to_string();
                        let version = lifecycle_state.0.begin_start(&action);
                        let mapping = mapping.clone();
                        let before_script_ast = mapping.script_hooks.before_script_ast.clone();
                        let after_script_ast = mapping.script_hooks.after_script_ast.clone();
                        let pointer_id = mapping.pointer_id;
                        let original_pos: Vec2 = mapping.position.into();
                        let random_offset =
                            Vec2::new(mapping.random_offset_x, mapping.random_offset_y);
                        let duration = Duration::from_millis(mapping.duration as u64);
                        let exec_ctx = make_mapping_execution_context(
                            &cs_tx_res,
                            &script_command_tx,
                            &shared_state,
                            mapping.id.clone(),
                            original_size,
                            cursor_pos.0,
                            mask_size.0,
                            mapping_state.get() == &MappingState::RawInput,
                            cursor_state.get() == &CursorState::Fps,
                        );
                        runtime.spawn_background_task(move |mut task_ctx| async move {
                            if let Err(e) = run_script_hook(&before_script_ast, &exec_ctx).await {
                                task_ctx
                                    .run_on_main_thread({
                                        let action = action.clone();
                                        move |main_ctx| {
                                            main_ctx
                                                .world
                                                .resource_mut::<RepeatTapLifecycleState>()
                                                .0
                                                .cancel_start(&action, version);
                                        }
                                    })
                                    .await;
                                log::error!("[RepeatTap] script hook runtime error: {:?}", e);
                                return;
                            }

                            let pending_release = task_ctx
                                .run_on_main_thread(move |main_ctx| {
                                    let start = main_ctx
                                        .world
                                        .resource_mut::<RepeatTapLifecycleState>()
                                        .0
                                        .finish_start(&action, version);
                                    let pending_release = match start {
                                        MappingLifecycleStart::Stale => return None,
                                        MappingLifecycleStart::Ready { pending_release } => {
                                            pending_release
                                        }
                                    };

                                    if pending_release.is_none() {
                                        let mut active_map =
                                            main_ctx.world.resource_mut::<ActiveRepeatTapMap>();
                                        active_map.0.insert(
                                            action.clone(),
                                            make_repeat_tap_timer(&mapping, original_size),
                                        );
                                    }

                                    pending_release
                                })
                                .await;

                            if let Some(release) = pending_release {
                                let random_pos = random_offset_vec2(original_pos, random_offset);
                                ControlMsgHelper::send_touch(
                                    &exec_ctx.cs_tx,
                                    MotionEventAction::Down,
                                    pointer_id,
                                    original_size,
                                    random_pos,
                                );
                                sleep(duration).await;
                                ControlMsgHelper::send_touch(
                                    &exec_ctx.cs_tx,
                                    MotionEventAction::Up,
                                    pointer_id,
                                    original_size,
                                    random_pos,
                                );

                                if !after_script_ast.empty {
                                    let mut after_exec_ctx = exec_ctx.clone();
                                    after_exec_ctx.cursor_pos = release.cursor_pos;
                                    after_exec_ctx.mask_size = release.mask_size;
                                    after_exec_ctx.raw_input_flag = release.raw_input_flag;
                                    after_exec_ctx.fps_mode_flag = release.fps_mode_flag;
                                    if let Err(e) =
                                        run_script_hook(&after_script_ast, &after_exec_ctx).await
                                    {
                                        log::error!(
                                            "[RepeatTap] script hook runtime error: {:?}",
                                            e
                                        );
                                    }
                                }
                            }
                        });
                    } else {
                        active_map.0.insert(
                            action.to_string(),
                            make_repeat_tap_timer(mapping, original_size),
                        );
                    }
                } else if ineffable.just_deactivated(action.ineff_continuous()) {
                    let released = active_map.0.remove(action.as_ref()).is_some();

                    if released {
                        lifecycle_state.0.clear_pending(action.as_ref());
                    } else if repeat_tap_has_before_hook(mapping) {
                        lifecycle_state.0.record_early_release(
                            action.as_ref(),
                            RepeatTapReleaseContext {
                                cursor_pos: cursor_pos.0,
                                mask_size: mask_size.0,
                                raw_input_flag: mapping_state.get() == &MappingState::RawInput,
                                fps_mode_flag: cursor_state.get() == &CursorState::Fps,
                            },
                        );
                    }

                    if released && repeat_tap_has_after_hook(mapping) {
                        let after_script_ast = mapping.script_hooks.after_script_ast.clone();
                        let exec_ctx = make_mapping_execution_context(
                            &cs_tx_res,
                            &script_command_tx,
                            &shared_state,
                            mapping.id.clone(),
                            active_mapping.original_size.into(),
                            cursor_pos.0,
                            mask_size.0,
                            mapping_state.get() == &MappingState::RawInput,
                            cursor_state.get() == &CursorState::Fps,
                        );
                        runtime.spawn_background_task(move |_ctx| async move {
                            if let Err(e) = run_script_hook(&after_script_ast, &exec_ctx).await {
                                log::error!("[RepeatTap] script hook runtime error: {:?}", e);
                            }
                        });
                    }
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MappingMultipleTapItem {
    pub position: Position,
    pub duration: u64,
    pub wait: u64,
}

#[derive(Debug, Clone)]
pub struct BindMappingMultipleTap {
    pub id: String,
    pub note: String,
    pub pointer_id: u64,
    pub items: Vec<MappingMultipleTapItem>,
    pub bind: ButtonBinding,
    pub input_binding: InputBinding,
    pub random_offset_x: f32,
    pub random_offset_y: f32,
    pub script_hooks: BindMappingScriptHooks,
}

impl From<MappingMultipleTap> for BindMappingMultipleTap {
    fn from(value: MappingMultipleTap) -> Self {
        Self {
            id: value.id,
            note: value.note,
            pointer_id: value.pointer_id,
            items: value.items,
            bind: value.bind.clone(),
            input_binding: PulseBinding::just_pressed(value.bind).0,
            random_offset_x: value.random_offset_x,
            random_offset_y: value.random_offset_y,
            script_hooks: value.script_hooks.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MappingMultipleTap {
    #[serde(default = "crate::mask::mapping::config::default_mapping_id")]
    pub id: String,
    pub note: String,
    pub pointer_id: u64,
    pub items: Vec<MappingMultipleTapItem>,
    pub bind: ButtonBinding,
    #[serde(
        default = "default_random_offset",
        serialize_with = "crate::mask::mapping::serde_float::serialize_f32_3dp"
    )]
    pub random_offset_x: f32,
    #[serde(
        default = "default_random_offset",
        serialize_with = "crate::mask::mapping::serde_float::serialize_f32_3dp"
    )]
    pub random_offset_y: f32,
    #[serde(default)]
    pub script_hooks: MappingScriptHooks,
}

impl ValidateMappingConfig for MappingMultipleTap {
    fn validate(&self) -> Result<(), String> {
        if self.items.is_empty() {
            return Err("MultipleTap's operation item list is empty".to_string());
        }
        self.script_hooks.validate()
    }
}

pub fn handle_multiple_tap(
    ineffable: Res<Ineffable>,
    active_mapping: Res<ActiveMappingConfig>,
    cs_tx_res: Res<ChannelSenderCS>,
    script_command_tx: Res<ScriptRuntimeCommandSender>,
    shared_state: Res<ScriptSharedState>,
    mask_size: Res<MaskSize>,
    cursor_pos: Res<CursorPosition>,
    mapping_state: Res<State<MappingState>>,
    cursor_state: Res<State<CursorState>>,
    runtime: ResMut<TokioTasksRuntime>,
) {
    if let Some(active_mapping) = &active_mapping.0 {
        for (action, mapping) in &active_mapping.mappings {
            if action.as_ref().starts_with("MultipleTap") {
                let mapping = mapping.as_ref_multipletap();
                if ineffable.just_pulsed(action.ineff_pulse()) {
                    let original_size: Vec2 = active_mapping.original_size.into();
                    let pointer_id = mapping.pointer_id;
                    let items = mapping.items.clone();
                    let random_offset = Vec2::new(mapping.random_offset_x, mapping.random_offset_y);
                    let hooks = mapping.script_hooks.clone();
                    let exec_ctx = make_mapping_execution_context(
                        &cs_tx_res,
                        &script_command_tx,
                        &shared_state,
                        mapping.id.clone(),
                        original_size,
                        cursor_pos.0,
                        mask_size.0,
                        mapping_state.get() == &MappingState::RawInput,
                        cursor_state.get() == &CursorState::Fps,
                    );
                    runtime.spawn_background_task(move |_ctx| async move {
                        let result = run_with_hooks(hooks, exec_ctx, move |ctx| async move {
                            for item in items {
                                let random_pos =
                                    random_offset_vec2(item.position.into(), random_offset);
                                sleep(Duration::from_millis(item.wait)).await;
                                ControlMsgHelper::send_touch(
                                    &ctx.cs_tx,
                                    MotionEventAction::Down,
                                    pointer_id,
                                    ctx.original_size,
                                    random_pos,
                                );
                                sleep(Duration::from_millis(item.duration)).await;
                                ControlMsgHelper::send_touch(
                                    &ctx.cs_tx,
                                    MotionEventAction::Up,
                                    pointer_id,
                                    ctx.original_size,
                                    random_pos,
                                );
                            }
                            Ok::<(), MappingExecutionError>(())
                        })
                        .await;
                        if let Err(e) = result {
                            log::error!("[MultipleTap] mapping execution error: {:?}", e);
                        }
                    });
                }
            }
        }
    }
}
