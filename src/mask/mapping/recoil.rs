//! Time-based, user-calibrated recoil assistance. Rates use original phone pixels/second.
use std::{str::FromStr, sync::{Arc, LazyLock, RwLock}};
use bevy::prelude::*;
use bevy_ineffable::prelude::Ineffable;
use serde::{Deserialize, Serialize};
use crate::{config::LocalConfig, mask::mask_command::MaskSize, scrcpy::constant::MotionEventAction,
    utils::ChannelSenderCS};
use super::{MappingState, binding::MergedButton, config::{ActiveMappingConfig, BindMappingType},
    cursor::{ActiveCursorFpsConfig, CursorFrameSet, CursorState}, device_pointer::DevicePointer,
    fire::ActiveFireMap, utils::ControlMsgHelper};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all="snake_case")]
pub enum RecoilMode { #[default] SameFinger, SeparateFinger }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RecoilStage { pub duration_ms: u32, pub x_per_second: f32, pub y_per_second: f32 }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RecoilProfile {
    pub id: String, pub name: String,
    #[serde(default)] pub notes: String,
    #[serde(default)] pub calibrated: bool,
    #[serde(default)] pub reference_source: String,
    #[serde(default)] pub reference_ratio: Option<f64>,
    pub delay_ms: u32, pub scale: f32, pub stages: Vec<RecoilStage>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RecoilConfig {
    pub enabled: bool, pub mode: RecoilMode, pub active_profile_id: String,
    pub toggle_key: String, pub next_profile_key: String,
    pub sync_weapon_keys: bool, pub slot1_profile_id: String, pub slot2_profile_id: String,
    pub pointer_id: u64, pub anchor_x: f32, pub anchor_y: f32,
    pub radius_x: f32, pub radius_y: f32, pub profiles: Vec<RecoilProfile>,
    pub reference_data: Option<serde_json::Value>,
}
impl Default for RecoilConfig {
    fn default() -> Self {
        Self { enabled:false, mode:RecoilMode::SameFinger, active_profile_id:"manual".into(),
            toggle_key:"F11".into(), next_profile_key:"F12".into(), sync_weapon_keys:true,
            slot1_profile_id:"manual".into(), slot2_profile_id:String::new(), pointer_id:90,
            anchor_x:0.7, anchor_y:0.5, radius_x:80.0, radius_y:80.0,
            profiles:vec![RecoilProfile { id:"manual".into(), name:"手动校准（测试曲线）".into(),
                notes:"仅供试滑动：30手机像素/秒，不是任何武器的实测参数".into(),
                calibrated:false, reference_source:String::new(), reference_ratio:None,
                delay_ms:0, scale:1.0, stages:vec![
                    RecoilStage { duration_ms:500, x_per_second:0.0, y_per_second:30.0 },
                    RecoilStage { duration_ms:4500, x_per_second:0.0, y_per_second:30.0 },
                ] }], reference_data:None }
    }
}
fn keyboard_key(name:&str) -> Option<KeyCode> {
    match MergedButton::from_str(name).ok()? { MergedButton::Keyboard(key) => Some(key), _ => None }
}
impl RecoilConfig {
    pub fn validate(&self) -> Result<(),String> {
        if self.profiles.is_empty() || self.profiles.len()>128 { return Err("压枪方案数量须为1～128".into()); }
        if keyboard_key(&self.toggle_key).is_none() || keyboard_key(&self.next_profile_key).is_none()
            || self.toggle_key==self.next_profile_key { return Err("开关和切换方案须绑定不同的键盘按键".into()); }
        if self.pointer_id>65535 { return Err("独立触点ID须在0～65535之间".into()); }
        for value in [self.anchor_x,self.anchor_y] {
            if !value.is_finite() || !(0.01..=0.99).contains(&value) { return Err("独立触点起点须在屏幕1%～99%内".into()); }
        }
        for value in [self.radius_x,self.radius_y] {
            if !value.is_finite() || !(1.0..=1000.0).contains(&value) { return Err("独立触点范围须在1～1000手机像素内".into()); }
        }
        let mut ids=std::collections::HashSet::new();
        for p in &self.profiles {
            if p.id.trim().is_empty() || p.name.trim().is_empty() || !ids.insert(&p.id) { return Err("方案ID须唯一，名称不能为空".into()); }
            if p.delay_ms>10000 || !p.scale.is_finite() || !(0.0..=20.0).contains(&p.scale)
                || p.stages.is_empty() || p.stages.len()>64 { return Err("方案延迟、总倍率或阶段数量无效".into()); }
            let mut total=0u64;
            for s in &p.stages {
                if s.duration_ms==0 || s.duration_ms>60000 || !s.x_per_second.is_finite() || !s.y_per_second.is_finite()
                    || s.x_per_second.abs()>10000.0 || s.y_per_second.abs()>10000.0 { return Err("每阶段须有正时长，X/Y速度须在±10000手机像素/秒内".into()); }
                total+=s.duration_ms as u64;
            }
            if total>60000 { return Err("单次射击的阶段总时长不能超过60秒".into()); }
            if p.reference_ratio.is_some_and(|v| !v.is_finite() || v<0.0) { return Err("参考系数无效".into()); }
        }
        for id in [&self.active_profile_id,&self.slot1_profile_id,&self.slot2_profile_id] {
            if !id.is_empty() && !ids.contains(id) { return Err(format!("压枪方案不存在：{id}")); }
        }
        if self.active_profile_id.is_empty() { return Err("请选择当前压枪方案".into()); }
        Ok(())
    }
    fn profile(&self)->&RecoilProfile {
        self.profiles.iter().find(|p| p.id==self.active_profile_id).unwrap_or(&self.profiles[0])
    }
}

/// Integrates phase boundaries exactly, independently of desktop frame rate.
fn integrated_offset(profile:&RecoilProfile, from:f64, to:f64)->Vec2 {
    let mut start=profile.delay_ms as f64/1000.0;
    let mut result=Vec2::ZERO;
    for stage in &profile.stages {
        let end=start+stage.duration_ms as f64/1000.0;
        let dt=(to.min(end)-from.max(start)).max(0.0) as f32;
        result+=Vec2::new(stage.x_per_second,stage.y_per_second)*dt*profile.scale;
        start=end;
    }
    result
}
fn total_duration(profile:&RecoilProfile)->f64 {
    (profile.delay_ms as f64+profile.stages.iter().map(|s|s.duration_ms as f64).sum::<f64>())/1000.0
}

#[derive(Resource,Default)] pub struct RecoilDelta(pub Vec2);
#[derive(Resource,Default)] pub struct RecoilRuntime {
    config:Option<Arc<RecoilConfig>>, elapsed:f64, active:bool, blocked_until_release:bool,
    second:Option<(u64,Vec2,Vec2)>, send_elapsed:f64,
    block_reason:Option<String>,
}
impl RecoilRuntime {
    pub fn keeps_view_touch(&self) -> bool {
        self.active && self.second.is_some()
    }
    fn release(&mut self,sender:&ChannelSenderCS) {
        if let Some((id,size,pos))=self.second.take() {
            ControlMsgHelper::send_touch(&sender.0,MotionEventAction::Up,id,size,pos);
        }
        self.active=false; self.elapsed=0.0; self.send_elapsed=0.0;
    }
}
#[derive(Serialize,Clone,Default,PartialEq)] pub struct RecoilStatus {
    pub enabled:bool, pub mode:String, pub profile:String, pub firing:bool, pub message:String,
}
static STATUS:LazyLock<RwLock<RecoilStatus>>=LazyLock::new(||RwLock::new(RecoilStatus::default()));
pub fn status()->RecoilStatus { STATUS.read().unwrap().clone() }
fn publish(cfg:&RecoilConfig,runtime:&RecoilRuntime,message:&str) {
    let next=RecoilStatus { enabled:cfg.enabled,mode:match cfg.mode {RecoilMode::SameFinger=>"同指",RecoilMode::SeparateFinger=>"双指"}.into(),
        profile:cfg.profile().name.clone(),firing:runtime.active,message:message.into() };
    if *STATUS.read().unwrap()!=next { *STATUS.write().unwrap()=next; }
}
pub struct RecoilPlugin;
impl Plugin for RecoilPlugin {
    fn build(&self,app:&mut App) {
        app.init_resource::<RecoilDelta>().init_resource::<RecoilRuntime>()
            .add_systems(Update,recoil_tick.in_set(CursorFrameSet::UpdatePosition).before(super::cursor::handle_cursor_fps))
            .add_systems(OnTransition { exited:MappingState::Normal, entered:MappingState::Stop },cleanup_recoil)
            .add_systems(OnTransition { exited:MappingState::RawInput, entered:MappingState::Stop },cleanup_recoil);
        app.add_systems(OnExit(CursorState::Fps),cleanup_recoil);
    }
}
pub fn cleanup_recoil(mut runtime:ResMut<RecoilRuntime>,mut delta:ResMut<RecoilDelta>,sender:Res<ChannelSenderCS>) {
    runtime.release(&sender); runtime.blocked_until_release=true; delta.0=Vec2::ZERO;
}
fn pointer_conflicts(id:u64,mappings:&super::config::BindMappingConfig)->bool {
    mappings.mappings.values().any(|m| match m {
        BindMappingType::SingleTap(v)=>v.pointer_id==id,
        BindMappingType::RepeatTap(v)=>v.pointer_id==id,
        BindMappingType::MultipleTap(v)=>v.pointer_id==id,
        BindMappingType::Swipe(v)=>v.pointer_id==id,
        BindMappingType::DirectionPad(v)=>v.pointer_id==id,
        BindMappingType::MouseCastSpell(v)=>v.pointer_id==id,
        BindMappingType::PadCastSpell(v)=>v.pointer_id==id,
        BindMappingType::Observation(v)=>v.pointer_id==id,
        BindMappingType::Fire(v)=>v.pointer_id==id,
        BindMappingType::Fps(v)=>v.pointer_id==id||v.touch_mode.another_pointer_id()==Some(id),
        _=>false,
    })
}
fn recoil_tick(time:Res<Time>,keys:Res<ButtonInput<KeyCode>>,window:Single<&Window>,
    mappings:Res<ActiveMappingConfig>,ineffable:Res<Ineffable>,fire:Res<ActiveFireMap>,
    pointer:Res<DevicePointer>,fps:Res<ActiveCursorFpsConfig>,mapping_state:Res<State<MappingState>>,
    cursor_state:Res<State<CursorState>>,sender:Res<ChannelSenderCS>,mask:Res<MaskSize>,
    mut delta:ResMut<RecoilDelta>,mut runtime:ResMut<RecoilRuntime>,resize:Res<crate::mask::MaskResizeState>) {
    delta.0=Vec2::ZERO;
    let mut cfg=LocalConfig::get_recoil();
    if window.focused && *mapping_state.get()!=MappingState::Stop {
        let mut edit=None;
        if keyboard_key(&cfg.toggle_key).is_some_and(|k|keys.just_pressed(k)) {
            let mut new=cfg.as_ref().clone(); new.enabled=!new.enabled; edit=Some(new);
        } else if keyboard_key(&cfg.next_profile_key).is_some_and(|k|keys.just_pressed(k)) {
            let mut new=cfg.as_ref().clone();
            let i=new.profiles.iter().position(|p|p.id==new.active_profile_id).unwrap_or(0);
            new.active_profile_id=new.profiles[(i+1)%new.profiles.len()].id.clone(); edit=Some(new);
        } else if cfg.sync_weapon_keys && !pointer.active {
            let selected=if keys.just_pressed(KeyCode::Digit1) { &cfg.slot1_profile_id }
                else if keys.just_pressed(KeyCode::Digit2) { &cfg.slot2_profile_id } else { "" };
            if !selected.is_empty() && selected!=cfg.active_profile_id {
                let mut new=cfg.as_ref().clone(); new.active_profile_id=selected.into(); edit=Some(new);
            }
        }
        if let Some(new)=edit { LocalConfig::set_recoil(new); cfg=LocalConfig::get_recoil(); }
    }
    let held=mappings.0.as_ref().is_some_and(|m|fire.is_firing_for_recoil(&ineffable,m));
    let free_view=mappings.0.as_ref().is_some_and(|m|m.mappings.iter().any(|(a,v)|
        matches!(v,BindMappingType::Observation(_))&&ineffable.is_active(a.ineff_continuous())));
    let allowed=cfg.enabled && window.focused && *mapping_state.get()==MappingState::Normal
        && *cursor_state.get()==CursorState::Fps && !pointer.active && !fps.ignore_fps_motion && !free_view
        && !resize.active() && mask.0.min_element()>0.0 && fps.original_size.min_element()>0.0;
    let changed=runtime.config.as_ref().is_some_and(|old|!Arc::ptr_eq(old,&cfg));
    if changed || !allowed || !held {
        runtime.release(&sender);
        if !held {runtime.blocked_until_release=false;runtime.block_reason=None;}
        else if changed || !allowed {runtime.blocked_until_release=true;runtime.block_reason=None;}
    }
    runtime.config=Some(cfg.clone());
    if !allowed || !held || runtime.blocked_until_release {
        let message=if !cfg.enabled {"已关闭"}
            else if *mapping_state.get()==MappingState::Stop {"请先连接并控制设备"}
            else if *cursor_state.get()!=CursorState::Fps {"请先进入FPS视角"}
            else if pointer.active {"手机指针状态，暂停补偿"}
            else if free_view {"自由视角状态，暂停补偿"}
            else if !window.focused {"控制窗口未聚焦，暂停补偿"}
            else if runtime.blocked_until_release {runtime.block_reason.as_deref().unwrap_or("请松开开火键后重新按下")}
            else {"待开火"};
        publish(&cfg,&runtime,message); return;
    }
    let profile=cfg.profile();
    runtime.active=true;
    let from=runtime.elapsed;
    // Do not repay a long scheduling stall as a large camera jump.
    let dt=time.delta_secs_f64().min(0.05);
    runtime.elapsed+=dt;
    let movement=integrated_offset(profile,from,runtime.elapsed);
    match cfg.mode {
        RecoilMode::SameFinger=>delta.0=movement,
        RecoilMode::SeparateFinger=>{
            if movement!=Vec2::ZERO {
                let size=fps.original_size;
                let anchor=Vec2::new(cfg.anchor_x,cfg.anchor_y)*size;
                let min=(anchor-Vec2::new(cfg.radius_x,cfg.radius_y)).max(Vec2::ZERO);
                let max=(anchor+Vec2::new(cfg.radius_x,cfg.radius_y)).min(size-Vec2::ONE);
                if runtime.second.is_none() {
                    if mappings.0.as_ref().is_some_and(|m|pointer_conflicts(cfg.pointer_id,m))
                        || ControlMsgHelper::touch_id_in_use(&sender.0,cfg.pointer_id) {
                        runtime.release(&sender);runtime.blocked_until_release=true;
                        runtime.block_reason=Some("独立触点ID冲突，请换一个ID，再松开重新开火".into());
                        publish(&cfg,&runtime,"独立触点ID冲突，请换一个ID");return;
                    }
                    ControlMsgHelper::send_touch(&sender.0,MotionEventAction::Down,cfg.pointer_id,size,anchor);
                    runtime.second=Some((cfg.pointer_id,size,anchor));
                }
                let (id,_,mut position)=runtime.second.unwrap();
                let next=position+movement;
                if next.x<min.x||next.x>max.x||next.y<min.y||next.y>max.y {
                    ControlMsgHelper::send_touch(&sender.0,MotionEventAction::Up,id,size,position);
                    ControlMsgHelper::send_touch(&sender.0,MotionEventAction::Down,id,size,anchor);
                    position=(anchor+movement).clamp(min,max);
                } else { position=next; }
                runtime.second=Some((id,size,position));
                runtime.send_elapsed+=dt;
                if runtime.send_elapsed>=1.0/120.0 {
                    ControlMsgHelper::send_touch(&sender.0,MotionEventAction::Move,cfg.pointer_id,size,position);
                    runtime.send_elapsed%=1.0/120.0;
                }
            }
        }
    }
    if runtime.elapsed>=total_duration(profile) {
        runtime.release(&sender);runtime.blocked_until_release=true;
    }
    publish(&cfg,&runtime,if profile.calibrated {"已校准方案"} else {"未校准：请在训练场调整"});
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn stopped_startup_does_not_require_the_control_channel_before_startup() {
        let mut app=App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .insert_state(MappingState::Stop).insert_state(CursorState::Normal)
            .add_plugins(RecoilPlugin);
        // State initialization runs before Startup creates the control channel.
        app.world_mut().run_schedule(StateTransition);
        assert!(!app.world().resource::<RecoilRuntime>().active);
    }
    #[test] fn leaving_fps_cleans_up_the_independent_touch_and_compensation() {
        let (tx,mut rx)=tokio::sync::broadcast::channel(8);
        let mut app=App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .insert_state(MappingState::Normal).insert_state(CursorState::Fps)
            .insert_resource(ChannelSenderCS(tx)).add_plugins(RecoilPlugin);
        app.world_mut().run_schedule(StateTransition);
        let size=Vec2::splat(1000.0);let pos=Vec2::splat(500.0);
        {
            let mut runtime=app.world_mut().resource_mut::<RecoilRuntime>();
            runtime.second=Some((90,size,pos));runtime.active=true;
        }
        app.world_mut().resource_mut::<RecoilDelta>().0=Vec2::ONE;
        app.world_mut().resource_mut::<NextState<CursorState>>().set(CursorState::Normal);
        app.world_mut().run_schedule(StateTransition);
        assert!(matches!(rx.try_recv().unwrap(),crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent{action:MotionEventAction::Up,pointer_id:90,..}));
        assert!(rx.try_recv().is_err());
        let runtime=app.world().resource::<RecoilRuntime>();
        assert!(!runtime.active && runtime.second.is_none() && runtime.blocked_until_release);
        assert_eq!(app.world().resource::<RecoilDelta>().0,Vec2::ZERO);
    }
    #[test] fn integrates_delay_and_phase_boundaries_independently_of_rate() {
        let mut p=RecoilConfig::default().profiles.remove(0);
        p.delay_ms=100;p.scale=2.0;
        p.stages=vec![RecoilStage{duration_ms:500,x_per_second:1.0,y_per_second:30.0},
            RecoilStage{duration_ms:500,x_per_second:-1.0,y_per_second:60.0}];
        let expected=integrated_offset(&p,0.0,2.0);
        assert!((expected-Vec2::new(0.0,90.0)).length()<0.001);
        for hz in [60,120,1000] {
            let mut sum=Vec2::ZERO;
            for i in 0..2*hz {sum+=integrated_offset(&p,i as f64/hz as f64,(i+1) as f64/hz as f64);}
            assert!((sum-expected).length()<0.01);
        }
        assert_eq!(integrated_offset(&p,0.0,0.05),Vec2::ZERO);
    }
    #[test] fn validates_profile_ids_ranges_and_negative_speed() {
        let mut c=RecoilConfig::default();assert!(c.validate().is_ok());
        c.profiles[0].stages[0].x_per_second=-2.0;assert!(c.validate().is_ok());
        c.active_profile_id="missing".into();assert!(c.validate().is_err());
        c.active_profile_id="manual".into();c.profiles[0].stages[0].duration_ms=0;assert!(c.validate().is_err());
    }
    #[test] fn independent_touch_is_released_once() {
        let(tx,mut rx)=tokio::sync::broadcast::channel(8);
        let sender=ChannelSenderCS(tx);let size=Vec2::splat(1000.0);let pos=Vec2::splat(500.0);
        ControlMsgHelper::send_touch(&sender.0,MotionEventAction::Down,90,size,pos);rx.try_recv().unwrap();
        let mut state=RecoilRuntime::default();state.second=Some((90,size,pos));state.active=true;
        state.release(&sender);state.release(&sender);
        assert!(matches!(rx.try_recv().unwrap(),crate::scrcpy::control_msg::ScrcpyControlMsg::InjectTouchEvent{action:MotionEventAction::Up,..}));
        assert!(rx.try_recv().is_err());assert!(!state.active);
    }
}
