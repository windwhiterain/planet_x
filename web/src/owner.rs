//! 启动者租约：盯住「谁把我拉起来的」，那个人一没，服务就自己退。
//!
//! 为什么需要它：Windows 没有「进程组一锅端」。agent 在后台 job 里跑
//! `cargo run -p planet_x_web`，job / daemon 被收掉时 `cargo` 死了，`planet_x_web.exe`
//! 却被系统放过，变成一个继续监听的孤儿——本仓真实发生过：一个 10:43 起的 exe 活到
//! 了 11:20 之后，而且它顺手把 `target\debug\planet_x_web.exe` 锁住，让后来那次重新
//! 编译只更新了 `deps\` 里的副本、换不掉顶层二进制（于是「我明明重编了」跑的还是老货）。
//!
//! 所以由**启动器**（`scripts/web.ps1`）把自己的 pid 通过 `PLANET_X_WEB_OWNER_PID`
//! 交给服务；服务开一个句柄等它结束（Windows）或轮询它是否还在（unix），一结束就
//! **立刻**退出。这里没有任何「空闲多久就退」的计时器：触发即退。

#[cfg(unix)]
use std::time::Duration;

/// 解析 `PLANET_X_WEB_OWNER_PID`：没设 / 空 / `0` / 不是数字一律当「没有租约」。
///
/// `0` 是「没设」而不是「pid 0」：Windows 的 pid 0 是 Idle 进程，永远在，写成 0
/// 最可能的意图是「关掉看护」，把它当成一个永远活着的主人等于把看护静默废掉。
pub fn parse_owner_pid(raw: Option<&str>) -> Option<u32> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    match s.parse::<u32>() {
        Ok(0) | Err(_) => None,
        Ok(pid) => Some(pid),
    }
}

/// 阻塞直到 `pid` 结束；它已经结束了就立刻返回。
///
/// 注意「打不开 / 查不到」一律算「已结束」：确认不了主人还活着，就不该继续保活——
/// 否则一个看不见的目标会把服务变成新的孤儿。
#[cfg(windows)]
pub fn wait_for_exit(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };
    // SAFETY: `OpenProcess` 只取一个同步句柄；失败返回空指针，下面据此提前返回。
    // `WaitForSingleObject(.., INFINITE)` 在这个句柄上一直等，句柄一 signal（进程结束）
    // 就返回；句柄随后关闭，不泄漏。
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return;
        }
        WaitForSingleObject(handle, INFINITE);
        CloseHandle(handle);
    }
}

/// unix 上等一个**不属于自己**的进程：没有现成接口，只能轮询 `kill(pid, 0)`。
///
/// 200ms 是「查得勤」不是「空闲超时」：主人一没，最多 200ms 就退。
#[cfg(unix)]
pub fn wait_for_exit(pid: u32) {
    while process_alive(pid) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: 0 号信号不投递任何东西，只做「这个 pid 还能不能寻址」的存在性检查。
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// 其它平台没有实现：不假装看护，直接返回（调用方会把退出请求当成「主人已没」）。
#[cfg(not(any(windows, unix)))]
pub fn wait_for_exit(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn owner_pid_parsing() {
        assert_eq!(parse_owner_pid(None), None);
        assert_eq!(parse_owner_pid(Some("")), None);
        assert_eq!(parse_owner_pid(Some("   ")), None);
        assert_eq!(parse_owner_pid(Some("0")), None, "0 = 没设，不是「盯住 Idle 进程」");
        assert_eq!(parse_owner_pid(Some("abc")), None);
        assert_eq!(parse_owner_pid(Some("-1")), None);
        assert_eq!(parse_owner_pid(Some(" 4242 ")), Some(4242));
    }

    /// 盯一个**已经不存在**的 pid 必须立刻返回，不能挂住——这是「启动器早退了」的
    /// 常见情形（脚本被 Ctrl+C、job 被收掉）。
    #[test]
    fn waiting_on_a_dead_pid_returns_at_once() {
        let start = std::time::Instant::now();
        // 一个几乎肯定不存在的 pid（Windows 上打不开句柄、unix 上 ESRCH）。
        wait_for_exit(0x7FFF_FFF0);
        assert!(start.elapsed() < Duration::from_secs(2), "死 pid 不该让看护挂住");
    }
}
