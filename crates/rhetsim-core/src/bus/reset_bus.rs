//! ResetBus — 复位信号总线

use crate::error::OmniResult;
use crate::unit::UnitId;
use std::sync::Arc;
use std::sync::{Condvar, Mutex};

/// 复位信号接收端
pub trait ResetSink: Send + Sync {
    /// 拉低复位信号
    fn assert_reset(&self) -> OmniResult<()>;
    /// 释放复位信号
    fn deassert_reset(&self) -> OmniResult<()>;
    /// 当前是否处于复位状态
    fn is_in_reset(&self) -> bool;
}

/// 一条复位线
pub struct ResetLine {
    /// 复位接收端
    pub sink: Arc<dyn ResetSink>,
    /// 复位线 ID
    pub line_id: String,
}

/// 就绪信号：Unit 上电并初始化完成后通知等待方
pub struct ReadySignal {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl ReadySignal {
    /// 创建未触发的就绪信号
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// 标记就绪并唤醒所有等待者
    pub fn set(&self) {
        let (lock, cvar) = &*self.inner;
        *lock.lock().expect("ready signal mutex poisoned") = true;
        cvar.notify_all();
    }

    /// 等待就绪，可指定超时
    pub fn wait(&self, timeout_ns: Option<u64>) -> OmniResult<()> {
        let (lock, cvar) = &*self.inner;
        let guard = lock.lock().expect("ready signal mutex poisoned");
        if *guard {
            return Ok(());
        }
        match timeout_ns {
            None => {
                let _guard = cvar.wait(guard).expect("ready signal mutex poisoned");
                Ok(())
            }
            Some(ns) => {
                let dur = std::time::Duration::from_nanos(ns);
                let (_guard, timed_out) = cvar
                    .wait_timeout(guard, dur)
                    .expect("ready signal mutex poisoned");
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
    fn default() -> Self {
        Self::new()
    }
}

/// 一条复位连接
pub struct ResetConnection {
    /// 复位源 Unit
    pub from: UnitId,
    /// 复位目标 Unit
    pub to: UnitId,
}

/// 复位信号总线
pub struct ResetBus {
    /// 总线 ID
    pub id: String,
    /// 已登记的复位连接
    pub connections: Vec<ResetConnection>,
    sinks: Vec<(UnitId, Arc<dyn ResetSink>)>,
}

impl ResetBus {
    /// 创建空的复位总线
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            connections: Vec::new(),
            sinks: Vec::new(),
        }
    }

    /// 注册一个复位接收端
    pub fn register_sink(&mut self, unit_id: UnitId, sink: Arc<dyn ResetSink>) {
        self.sinks.push((unit_id, sink));
    }

    /// 拉低所有接收端的复位信号
    pub fn assert_all(&self) -> OmniResult<()> {
        for (_, sink) in &self.sinks {
            sink.assert_reset()?;
        }
        Ok(())
    }

    /// 释放指定接收端的复位信号
    pub fn deassert(&self, target: &UnitId) -> OmniResult<()> {
        self.sinks
            .iter()
            .find(|(id, _)| id == target)
            .map(|(_, s)| s.deassert_reset())
            .unwrap_or(Err(crate::error::OmniError::UnitNotFound(
                target.to_string(),
            )))
    }

    /// 释放所有接收端的复位信号
    pub fn deassert_all(&self) -> OmniResult<()> {
        for (_, sink) in &self.sinks {
            sink.deassert_reset()?;
        }
        Ok(())
    }
}
