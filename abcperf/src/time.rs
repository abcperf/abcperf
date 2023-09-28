use std::{
    ops::{Add, AddAssign},
    time::{Duration, Instant, SystemTime},
};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(into = "SystemTime", try_from = "SystemTime")]
pub(crate) struct SharedTime {
    local: Instant,
    shared: SystemTime,
}

impl SharedTime {
    pub(crate) fn synced_now() -> Self {
        let local = Instant::now();
        let shared = SystemTime::now();
        Self { local, shared }
    }

    pub(crate) fn unix(self) -> u64 {
        self.shared
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("we live after the UNIX EPOCH")
            .as_secs()
    }

    pub(crate) fn elapsed(self) -> Duration {
        self.local.elapsed()
    }
}

impl Add<Duration> for SharedTime {
    type Output = Self;

    fn add(self, duration: Duration) -> Self::Output {
        Self {
            local: self.local.checked_add(duration).unwrap(),
            shared: self.shared.checked_add(duration).unwrap(),
        }
    }
}

impl AddAssign<Duration> for SharedTime {
    fn add_assign(&mut self, duration: Duration) {
        *self = *self + duration;
    }
}

impl TryFrom<SystemTime> for SharedTime {
    type Error = anyhow::Error;

    fn try_from(shared: SystemTime) -> Result<Self, Self::Error> {
        let now_local = Instant::now();
        let now_shared = SystemTime::now();

        let local = match now_shared.duration_since(shared) {
            Ok(elapsed) => now_local
                .checked_sub(elapsed)
                .ok_or_else(|| anyhow!("negative instant"))?,
            Err(_) => {
                let elapsed = shared.duration_since(now_shared)?;
                now_local
                    .checked_add(elapsed)
                    .ok_or_else(|| anyhow!("overflow"))?
            }
        };
        Ok(Self { local, shared })
    }
}

impl From<SharedTime> for SystemTime {
    fn from(time: SharedTime) -> Self {
        time.shared
    }
}

impl From<SharedTime> for Instant {
    fn from(time: SharedTime) -> Self {
        time.local
    }
}

impl From<SharedTime> for tokio::time::Instant {
    fn from(time: SharedTime) -> Self {
        time.local.into()
    }
}

impl AsRef<SystemTime> for SharedTime {
    fn as_ref(&self) -> &SystemTime {
        &self.shared
    }
}

impl AsRef<Instant> for SharedTime {
    fn as_ref(&self) -> &Instant {
        &self.local
    }
}
