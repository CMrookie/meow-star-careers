//! 简易进程内登录防爆破限流器：
//! - 以「邮箱|客户端IP」为键，连续失败 N 次进入冻结窗口；
//! - 进程内 Mutex 实现（单实例足够；多实例部署请替换为 Redis 等共享存储）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 同一键在窗口期内允许的失败次数
const MAX_FAILURES: u32 = 5;
/// 计数窗口：窗口期内达到上限则冻结
const WINDOW: Duration = Duration::from_secs(15 * 60);
/// 冻结时长
const BLOCK: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Default)]
pub struct LoginLimiter {
    inner: Arc<Mutex<HashMap<String, Entry>>>,
}

#[derive(Default, Clone, Copy)]
struct Entry {
    failures: u32,
    first_failure: Option<Instant>,
    blocked_until: Option<Instant>,
}

impl LoginLimiter {
    /// 该键当前是否被冻结；顺带清理过期条目
    pub fn blocked(&self, key: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        let now = Instant::now();

        match map.get(key) {
            None => false,
            Some(entry) => {
                if let Some(until) = entry.blocked_until {
                    if now < until {
                        true
                    } else {
                        map.remove(key);
                        false
                    }
                } else if entry
                    .first_failure
                    .map(|t| now.duration_since(t) > WINDOW)
                    .unwrap_or(false)
                {
                    map.remove(key);
                    false
                } else {
                    false
                }
            }
        }
    }

    /// 记录一次失败；达到上限即冻结
    pub fn record_failure(&self, key: &str) {
        let mut map = self.inner.lock().unwrap();
        let now = Instant::now();

        let entry = map.entry(key.to_string()).or_default();
        // 滑动窗口：距首次失败超过窗口则重新计数
        if let Some(first) = entry.first_failure {
            if now.duration_since(first) > WINDOW {
                *entry = Entry::default();
            }
        }
        entry.failures += 1;
        if entry.first_failure.is_none() {
            entry.first_failure = Some(now);
        }
        if entry.failures >= MAX_FAILURES {
            entry.blocked_until = Some(now + BLOCK);
        }
    }

    /// 登录成功后清空该键计数
    pub fn clear(&self, key: &str) {
        self.inner.lock().unwrap().remove(key);
    }
}
