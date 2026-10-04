//! Latest-position sampling for FPS touches; gesture boundaries are never dropped.
use std::{collections::HashMap, sync::{LazyLock, Mutex}, time::{Duration, Instant}};
use bevy::prelude::Vec2;
use tokio::sync::broadcast;
use crate::scrcpy::{constant::MotionEventAction, control_msg::ScrcpyControlMsg};
use super::utils::ControlMsgHelper;

const INTERVAL:Duration=Duration::from_nanos(1_000_000_000/120);
#[derive(Clone,Copy)]
struct Point { size:Vec2, pos:Vec2 }
impl Point {
    fn key(self)->(u16,u16,i32,i32) { (self.size.x as u16,self.size.y as u16,self.pos.x as i32,self.pos.y as i32) }
}
#[derive(Default)]
struct Stream { last:Option<Point>, sent_at:Option<Instant>, pending:Option<Point> }
impl Stream {
    fn submit(&mut self,point:Point,now:Instant)->Option<Point> {
        // Returning to the last transmitted position also cancels any unsent movement.
        self.pending=if self.last.is_some_and(|p|p.key()==point.key()) { None } else { Some(point) };
        self.due(now)
    }
    fn due(&mut self,now:Instant)->Option<Point> {
        if self.sent_at.is_some_and(|at|now.duration_since(at)<INTERVAL) { return None; }
        self.take(now)
    }
    fn take(&mut self,now:Instant)->Option<Point> {
        let point=self.pending.take()?;
        self.last=Some(point);self.sent_at=Some(now);Some(point)
    }
}
struct Session { sender:broadcast::WeakSender<ScrcpyControlMsg>, streams:HashMap<u64,Stream> }
static SESSIONS:LazyLock<Mutex<Vec<Session>>>=LazyLock::new(||Mutex::new(Vec::new()));
fn emit(tx:&broadcast::Sender<ScrcpyControlMsg>,action:MotionEventAction,id:u64,point:Point) {
    super::fps_diagnostics::generated(action,id,point.pos);
    ControlMsgHelper::send_touch(tx,action,id,point.size,point.pos);
}
pub(super) fn send(tx:&broadcast::Sender<ScrcpyControlMsg>,action:MotionEventAction,id:u64,size:Vec2,pos:Vec2) {
    let now=Instant::now();let point=Point{size,pos};
    let mut sessions=SESSIONS.lock().unwrap();
    sessions.retain(|s|s.sender.upgrade().is_some());
    let index=sessions.iter().position(|s|s.sender.upgrade().is_some_and(|sender|sender.same_channel(tx)))
        .unwrap_or_else(||{sessions.push(Session{sender:tx.downgrade(),streams:HashMap::new()});sessions.len()-1});
    let streams=&mut sessions[index].streams;
    if action==MotionEventAction::Move {
        if let Some(latest)=streams.entry(id).or_default().submit(point,now) { emit(tx,action,id,latest); }
    } else {
        // Finish the latest movement before lifting; do not replay intermediate positions.
        if action==MotionEventAction::Up {
            if let Some(mut stream)=streams.remove(&id) {
                if let Some(latest)=stream.take(now) { emit(tx,MotionEventAction::Move,id,latest); }
            }
        } else if action==MotionEventAction::Down {
            streams.insert(id,Stream{last:Some(point),..Default::default()});
        } else { streams.remove(&id); }
        emit(tx,action,id,point);
    }
}
pub(super) fn flush(tx:&broadcast::Sender<ScrcpyControlMsg>) {
    let now=Instant::now();let mut sessions=SESSIONS.lock().unwrap();
    sessions.retain(|s|s.sender.upgrade().is_some());
    if let Some(session)=sessions.iter_mut().find(|s|s.sender.upgrade().is_some_and(|sender|sender.same_channel(tx))) {
        for (&id,stream) in &mut session.streams {
            if let Some(point)=stream.due(now) { emit(tx,MotionEventAction::Move,id,point); }
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn point(x:f32)->Point { Point{size:Vec2::splat(1000.0),pos:Vec2::new(x,500.0)} }
    #[test] fn high_rate_motion_keeps_latest_position_without_a_replay_queue() {
        let start=Instant::now();let mut stream=Stream::default();let mut count=0;
        for i in 0..1001 {
            if stream.submit(point(i as f32),start+Duration::from_millis(i)).is_some(){count+=1;}
        }
        assert!(count<=120);
        let last=stream.due(start+Duration::from_millis(1010)).unwrap();
        assert_eq!(last.pos.x,1000.0);
        assert!(stream.due(start+Duration::from_secs(2)).is_none());
    }
    #[test] fn subpixel_duplicates_are_filtered_and_return_cancels_pending_motion() {
        let now=Instant::now();let mut stream=Stream::default();
        assert!(stream.submit(point(5.0),now).is_some());
        assert!(stream.submit(point(5.9),now+INTERVAL).is_none());
        assert!(stream.submit(point(8.0),now+Duration::from_millis(1)).is_none());
        assert!(stream.submit(point(5.1),now+Duration::from_millis(2)).is_none());
        assert!(stream.due(now+Duration::from_secs(1)).is_none());
    }
    #[test] fn up_flushes_latest_move_once_and_clears_pending_state() {
        let (tx,mut rx)=broadcast::channel(16);
        send(&tx,MotionEventAction::Down,0,point(0.0).size,point(0.0).pos);
        send(&tx,MotionEventAction::Move,0,point(1.0).size,point(1.0).pos);
        send(&tx,MotionEventAction::Move,0,point(2.0).size,point(2.0).pos);
        send(&tx,MotionEventAction::Up,0,point(2.0).size,point(2.0).pos);
        flush(&tx);
        let mut events=Vec::new();
        while let Ok(ScrcpyControlMsg::InjectTouchEvent{action,x,..})=rx.try_recv(){events.push((action,x));}
        assert_eq!(events,vec![(MotionEventAction::Down,0),(MotionEventAction::Move,1),
            (MotionEventAction::Move,2),(MotionEventAction::Up,2)]);
    }
}
