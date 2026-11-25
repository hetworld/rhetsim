//! ResetBus — 复位信号总线

use std::sync::Arc;
use std::sync::{Mutex, Condvar};
use crate::error::OmniResult;
use crate::unit::UnitId;

/// 复位信号接收端
pub trait ResetSink: Send + Sync {
    fn assert_reset(&self)   -> OmniResult<()>;
    fn deassert_reset(&self) -> OmniResult<()>;
    fn is_in_reset(&self)    -> bool;
}

/// 一条复位线
pub struct ResetLine {
    pub sink:    Arc<dyn ResetSink>,
    pub line_id: String,
}

/// 就绪信号：Unit 上电并初始化完成后通知等待方
pub struct ReadySignal {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl ReadySignal {
    pub fn new() -> Self {
        Self { inner: Arc::new((Mutex::new(false), Condvar::new())) }
    }

    pub fn set(&self) {
        let (lock, cvar) = &*self.inner;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }

    pub fn wait(&self, timeout_ns: Option<u64>) -> OmniResult<()> {
        let (lock, cvar) = &*self.inner;
        let guard = lock.lock().unwrap();
        if *guard { return Ok(()); }
        match timeout_ns {
            None => { let _ = cvar.wait(guard); Ok(()) }
            Some(ns) => {
                let dur = std::time::Duration::from_nanos(ns);
                let (_, timed_out) = cvar.wait_timeout(guard, dur).unwrap();
                if timed_out.timed_out() {
                    Err(crate::error::OmniError::Timeout("ready signal".into()))
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl Default for ReadySignal {
    fn default() -> Self { Self::new() }
}

pub struct ResetConnection {
    pub from: UnitId,
    pub to:   UnitId,
}

pub struct ResetBus {
    pub id:          String,
    pub connections: Vec<ResetConnection>,
    sinks:           Vec<(UnitId, Arc<dyn ResetSink>)>,
}

impl ResetBus {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), connections: Vec::new(), sinks: Vec::new() }
    }

    pub fn register_sink(&mut self, unit_id: UnitId, sink: Arc<dyn ResetSink>) {
        self.sinks.push((unit_id, sink));
    }

    pub fn assert_all(&self) -> OmniResult<()> {
        for (_, sink) in &self.sinks { sink.assert_reset()?; }
        Ok(())
    }

    pub fn deassert(&self, target: &UnitId) -> OmniResult<()> {
        self.sinks.iter()
            .find(|(id, _)| id == target)
            .map(|(_, s)| s.deassert_reset())
            .unwrap_or(Err(crate::error::OmniError::UnitNotFound(target.to_string())))
    }

    pub fn deassert_all(&self) -> OmniResult<()> {
        for (_, sink) in &self.sinks { sink.deassert_reset()?; }
        Ok(())
    }
}
