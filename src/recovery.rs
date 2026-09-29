use std::time::Duration;

// CLOCK_BOOTTIME includes suspend; CLOCK_MONOTONIC does not. Their difference
// detects resume even if USB nodes survive and the input readers look healthy.
// Unlike wall time this is unaffected by NTP or manual clock adjustments.
#[derive(Default)]
pub struct ResumeDetector {
    previous: Option<Duration>,
}

impl ResumeDetector {
    pub fn resumed(&mut self) -> bool {
        let Some(offset) = suspend_offset() else {
            return false;
        };
        self.observe(offset)
    }

    fn observe(&mut self, offset: Duration) -> bool {
        let previous = self.previous.replace(offset);
        previous
            .is_some_and(|previous| offset.saturating_sub(previous) > Duration::from_millis(100))
    }
}

fn suspend_offset() -> Option<Duration> {
    fn read_clock(id: libc::clockid_t) -> Option<Duration> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(id, &mut time) } != 0 {
            return None;
        }
        Some(Duration::new(
            time.tv_sec.try_into().ok()?,
            time.tv_nsec.try_into().ok()?,
        ))
    }
    let monotonic = read_clock(libc::CLOCK_MONOTONIC)?;
    let boot = read_clock(libc::CLOCK_BOOTTIME)?;
    Some(boot.saturating_sub(monotonic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_resume_once_even_when_device_nodes_survive() {
        let mut detector = ResumeDetector::default();
        assert!(!detector.observe(Duration::from_secs(20)));
        assert!(!detector.observe(Duration::from_secs(20)));
        assert!(detector.observe(Duration::from_secs(3620)));
        assert!(!detector.observe(Duration::from_secs(3620)));
        assert!(detector.observe(Duration::from_secs(3621)));
    }

    #[test]
    fn ignores_sampling_jitter_and_backwards_offsets() {
        let mut detector = ResumeDetector::default();
        assert!(!detector.observe(Duration::from_secs(2)));
        assert!(!detector.observe(Duration::from_millis(2001)));
        assert!(!detector.observe(Duration::from_secs(2)));
    }

    #[test]
    fn linux_suspend_clock_is_available() {
        assert!(suspend_offset().is_some());
    }
}
