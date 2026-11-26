use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use sysinfo::{ProcessesToUpdate, System};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
    QUNS_RUNNING_D3D_FULL_SCREEN,
};

use crate::error::{AppError, AppResult};

pub fn compute_effective_limit(
    base_limit_bps: Option<u64>,
    throttle_active: bool,
    percent: u8,
    fallback_unthrottled_bps: u64,
) -> Option<u64> {
    if !throttle_active {
        return base_limit_bps.filter(|&v| v > 0);
    }
    let pct = (percent as u64).clamp(1, 100);
    let baseline = base_limit_bps
        .filter(|&v| v > 0)
        .unwrap_or(fallback_unthrottled_bps);
    let throttled = ((baseline as u128 * pct as u128) / 100) as u64;
    Some(throttled.max(16 * 1024))
}

pub fn matches_target_process(process_name: &str, targets: &[String]) -> bool {
    let norm_proc = process_name.trim().to_ascii_lowercase();
    let norm_proc_no_ext = norm_proc.strip_suffix(".exe").unwrap_or(&norm_proc);
    targets.iter().any(|t| {
        let target = t.trim().to_ascii_lowercase();
        if target.is_empty() {
            return false;
        }
        let target_no_ext = target.strip_suffix(".exe").unwrap_or(&target);
        norm_proc == target || norm_proc_no_ext == target_no_ext
    })
}

pub async fn detect_throttle_trigger(
    targets: Vec<String>,
    check_fullscreen: bool,
) -> AppResult<Option<String>> {
    tokio::task::spawn_blocking(move || -> AppResult<Option<String>> {
        if check_fullscreen {
            if let Some(reason) = query_fullscreen_state() {
                return Ok(Some(reason));
            }
        }

        if !targets.is_empty() {
            let mut sys = System::new();
            sys.refresh_processes(ProcessesToUpdate::All, true);
            for proc_info in sys.processes().values() {
                let name = proc_info.name().to_string_lossy();
                if matches_target_process(&name, &targets) {
                    return Ok(Some(format!("Process active: {name}")));
                }
            }
        }

        Ok(None)
    })
    .await
    .map_err(|e| AppError::Other(format!("Auto-throttle scan panicked: {e}")))?
}

fn query_fullscreen_state() -> Option<String> {
    // SAFETY: SHQueryUserNotificationState is a thread-safe Win32 Shell API with no pointers passed by caller.
    let state = unsafe { SHQueryUserNotificationState() }.ok()?;
    if state == QUNS_RUNNING_D3D_FULL_SCREEN {
        Some("Fullscreen D3D app".to_string())
    } else if state == QUNS_PRESENTATION_MODE {
        Some("Presentation mode".to_string())
    } else if state == QUNS_BUSY {
        Some("Fullscreen or busy app".to_string())
    } else {
        None
    }
}

pub fn new_stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

pub fn is_stopped(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_effective_limit_correctly() {
        assert_eq!(compute_effective_limit(None, false, 20, 5_000_000), None);
        assert_eq!(
            compute_effective_limit(Some(10_000_000), false, 20, 5_000_000),
            Some(10_000_000)
        );
        assert_eq!(
            compute_effective_limit(Some(10_000_000), true, 20, 5_000_000),
            Some(2_000_000)
        );
        assert_eq!(
            compute_effective_limit(None, true, 20, 5_000_000),
            Some(1_000_000)
        );
    }

    #[test]
    fn matches_processes_case_insensitively_with_or_without_exe() {
        let targets = vec!["Zoom.exe".to_string(), "discord".to_string()];
        assert!(matches_target_process("zoom.exe", &targets));
        assert!(matches_target_process("DISCORD.EXE", &targets));
        assert!(!matches_target_process("notepad.exe", &targets));
    }
}
