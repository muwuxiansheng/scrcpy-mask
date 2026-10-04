//! Opt-in, one-second FPS input summaries. No per-event logging or input changes.
use std::{collections::HashMap, sync::{Mutex, OnceLock}, time::Instant};
use bevy::{input::mouse::{AccumulatedMouseMotion, MouseMotion}, prelude::*};
use super::{cursor::{ActiveCursorFpsConfig, CursorState}, device_pointer::DevicePointer, MappingState, mask_not_resizing};
use crate::scrcpy::{constant::MotionEventAction, control_msg::ScrcpyControlMsg};

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SCRCPY_MASK_FPS_DIAGNOSTICS").is_ok_and(|v| v == "1"))
}
struct Counters {
    start: Instant,
    frames: u32,
    moving: u32,
    mouse_events: usize,
    frame_ms: Vec<f32>,
    generated: u32,
    sent: u32,
    wire_ms: Vec<f32>,
    repeated: u32,
    quant_error: f32,
    recenters: u32,
    idle_releases: u32,
    lagged: u64,
}
impl Default for Counters {
    fn default() -> Self { Self { start: Instant::now(), frames: 0, moving: 0, mouse_events: 0,
        frame_ms: Vec::new(), generated: 0, sent: 0, wire_ms: Vec::new(), repeated: 0,
        quant_error: 0.0, recenters: 0, idle_releases: 0, lagged: 0 } }
}
#[derive(Default)]
struct Metrics {
    active: bool,
    ids: Vec<u64>,
    previous_frame: Option<Instant>,
    previous_wire: Option<Instant>,
    previous_position: HashMap<u64, (i32,i32)>,
    counters: Counters,
}
fn metrics() -> &'static Mutex<Metrics> {
    static METRICS: OnceLock<Mutex<Metrics>> = OnceLock::new();
    METRICS.get_or_init(|| Mutex::new(Metrics::default()))
}
fn distribution(values: &mut [f32]) -> (f32,f32,f32) {
    if values.is_empty() { return (0.0,0.0,0.0); }
    values.sort_by(f32::total_cmp);
    (values[values.len()/2], values[((values.len()-1) as f32 * 0.95).ceil() as usize], values[values.len()-1])
}
fn report(m: &mut Metrics) {
    let c = &mut m.counters;
    if c.frames == 0 { return; }
    let seconds = c.start.elapsed().as_secs_f32().max(0.001);
    let (f50,f95,fmax) = distribution(&mut c.frame_ms);
    let (w50,w95,wmax) = distribution(&mut c.wire_ms);
    log::info!("[FPS_DIAG] span_s={seconds:.2} frames={} moving_frames={} received_mouse_events={} mouse_events_hz={:.1} frame_ms_p50={f50:.2} p95={f95:.2} max={fmax:.2} generated_moves={} wire_moves={} wire_gap_ms_p50={w50:.2} p95={w95:.2} max={wmax:.2} repeated_integer_moves={} max_quant_error_px={:.3} recenters={} idle_releases={} channel_lagged={}",
        c.frames, c.moving, c.mouse_events, c.mouse_events as f32 / seconds, c.generated, c.sent,
        c.repeated, c.quant_error, c.recenters, c.idle_releases, c.lagged);
    m.counters = Counters::default();
}
pub fn capture_frame(
    mut events: MessageReader<MouseMotion>, motion: Res<AccumulatedMouseMotion>,
    cursor: Res<State<CursorState>>, mapping: Res<State<MappingState>>,
    pointer: Res<DevicePointer>, fps: Res<ActiveCursorFpsConfig>, window: Single<&Window>,
    resize: Res<crate::mask::MaskResizeState>,
) {
    let count = events.read().count();
    if !enabled() { return; }
    let active = *cursor.get() == CursorState::Fps && *mapping.get() == MappingState::Normal
        && !pointer.active && !fps.ignore_fps_motion && window.focused && mask_not_resizing(resize);
    let mut m = metrics().lock().unwrap();
    if !active {
        if m.active { report(&mut m); *m = Metrics::default(); }
        return;
    }
    if !m.active {
        *m = Metrics::default(); m.active = true;
        log::info!("[FPS_DIAG] START sensitivity=({:.3},{:.3}) profile=({:.0},{:.0}) max_offset=({:.1},{:.1}) mode={:?}; wire gaps include idle pauses, not game response time", fps.sensitivity.x, fps.sensitivity.y, fps.original_size.x, fps.original_size.y, fps.max_offset.x, fps.max_offset.y, fps.touch_mode);
    }
    m.ids = vec![fps.pointer_id];
    if let Some(id) = fps.touch_mode.another_pointer_id() { m.ids.push(id); }
    let now = Instant::now();
    if let Some(previous) = m.previous_frame { m.counters.frame_ms.push(now.duration_since(previous).as_secs_f32()*1000.0); }
    m.previous_frame = Some(now);
    m.counters.frames += 1; m.counters.mouse_events += count;
    if motion.delta != Vec2::ZERO { m.counters.moving += 1; }
    if m.counters.start.elapsed().as_secs_f32() >= 1.0 { report(&mut m); }
}
pub fn generated(action: MotionEventAction, id: u64, position: Vec2) {
    if !enabled() { return; }
    let mut m = metrics().lock().unwrap();
    if !m.active { return; }
    let integer = (position.x as i32,position.y as i32);
    if action == MotionEventAction::Move {
        m.counters.generated += 1;
        if m.previous_position.get(&id) == Some(&integer) { m.counters.repeated += 1; }
        m.counters.quant_error = m.counters.quant_error.max((position - Vec2::new(integer.0 as f32, integer.1 as f32)).abs().max_element());
    }
    m.previous_position.insert(id,integer);
    if action == MotionEventAction::Up { m.previous_position.remove(&id); }
}
pub fn recenter() { if enabled() { let mut m=metrics().lock().unwrap(); if m.active {m.counters.recenters+=1;} } }
pub fn idle_release() { if enabled() { let mut m=metrics().lock().unwrap(); if m.active {m.counters.idle_releases+=1;} } }
pub fn wire_written(id: u64, action: MotionEventAction) {
    if !enabled() { return; }
    let mut m=metrics().lock().unwrap();
    if !m.active || !m.ids.contains(&id) { return; }
    if action == MotionEventAction::Move {
        let now=Instant::now();
        if let Some(previous)=m.previous_wire {m.counters.wire_ms.push(now.duration_since(previous).as_secs_f32()*1000.0);}
        m.previous_wire=Some(now); m.counters.sent+=1;
    } else { m.previous_wire=None; }
}
pub fn lagged(count: u64) { if enabled() {let mut m=metrics().lock().unwrap(); if m.active {m.counters.lagged+=count;}} }
pub fn packet(msg: &ScrcpyControlMsg) -> Option<(u64,MotionEventAction)> {
    if !enabled() { return None; }
    if let ScrcpyControlMsg::InjectTouchEvent {pointer_id,action,..}=msg { Some((*pointer_id,*action)) } else {None}
}
