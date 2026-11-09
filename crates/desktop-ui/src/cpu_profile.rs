//! Low-frequency CPU sampling, attributed only to threads owned by this PID.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, LocalFree},
        System::Threading::{
            GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, GetProcessIdOfThread,
            GetProcessTimes, GetThreadDescription, GetThreadTimes, OpenThread,
            THREAD_QUERY_LIMITED_INFORMATION,
        },
    },
    core::{BOOL, Result},
};

// Minimal ToolHelp ABI avoids enabling another broad windows-rs feature for
// three functions. Layout/signatures match Windows SDK THREADENTRY32.
#[repr(C)]
#[derive(Default)]
struct ThreadEntry {
    size: u32,
    usage: u32,
    id: u32,
    owner: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Thread32First(snapshot: HANDLE, entry: *mut ThreadEntry) -> BOOL;
    fn Thread32Next(snapshot: HANDLE, entry: *mut ThreadEntry) -> BOOL;
}
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Times {
    created: u64,
    kernel: u64,
    user: u64,
}
fn ticks(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}
fn times(handle: HANDLE, process: bool) -> Result<Times> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: valid borrowed process/thread handles, and writable out parameters.
    unsafe {
        if process {
            GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)?;
        } else {
            GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)?;
        }
    }
    Ok(Times {
        created: ticks(created),
        kernel: ticks(kernel),
        user: ticks(user),
    })
}
fn name(handle: HANDLE, id: u32, ui: u32) -> String {
    // GetThreadDescription returns a separately allocated UTF-16 string.
    let description = unsafe { GetThreadDescription(handle) }
        .ok()
        .map(|text| unsafe {
            let name = text.to_string().unwrap_or_default();
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            name
        })
        .unwrap_or_default();
    if !description.is_empty() {
        description
    } else if id == ui {
        "desktop-ui".into()
    } else {
        "unnamed".into()
    }
}
struct ThreadSample {
    times: Times,
    name: String,
}
fn threads(
    pid: u32,
    ui: u32,
    previous: &HashMap<u32, ThreadSample>,
) -> Result<HashMap<u32, ThreadSample>> {
    // TH32CS_SNAPTHREAD enumerates IDs; filter ownership BEFORE opening a handle.
    let snapshot = unsafe { CreateToolhelp32Snapshot(0x00000004, 0) };
    if snapshot.is_invalid() {
        return Err(windows::core::Error::from_thread());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry = ThreadEntry {
        size: std::mem::size_of::<ThreadEntry>() as u32,
        ..Default::default()
    };
    let mut rows = HashMap::new();
    let mut valid = unsafe { Thread32First(snapshot.0, &mut entry) }.as_bool();
    while valid {
        if entry.owner == pid
            && let Ok(handle) =
                unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, entry.id) }
        {
            let handle = OwnedHandle(handle);
            // Threads may exit during enumeration: skip failed reads safely.
            // TIDs can be reused after the snapshot. Recheck live ownership
            // before reading timings so another instance cannot be attributed here.
            if unsafe { GetProcessIdOfThread(handle.0) } == pid
                && let Ok(times) = times(handle.0, false)
            {
                let name = previous
                    .get(&entry.id)
                    .filter(|p| p.times.created == times.created)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| name(handle.0, entry.id, ui));
                rows.insert(entry.id, ThreadSample { times, name });
            }
        }
        entry.size = std::mem::size_of::<ThreadEntry>() as u32;
        valid = unsafe { Thread32Next(snapshot.0, &mut entry) }.as_bool();
    }
    Ok(rows)
}
fn delta(new: Times, old: Times) -> (f64, f64) {
    (
        new.user.saturating_sub(old.user) as f64 / 10000.,
        new.kernel.saturating_sub(old.kernel) as f64 / 10000.,
    )
}
fn percentages(cpu_ms: f64, elapsed_ms: f64, processors: usize) -> (f64, f64) {
    let one_core = cpu_ms / elapsed_ms.max(0.001) * 100.;
    (one_core / processors.max(1) as f64, one_core)
}
#[derive(Clone, Copy)]
pub(crate) enum UiStage {
    Update,
    Draw,
    Present,
}
pub(crate) struct CpuStamp(Times);
#[derive(Clone, Copy, Default)]
struct StageCpu {
    user_ms: f64,
    kernel_ms: f64,
    samples: u64,
}

pub(crate) struct CpuProfiler {
    pid: u32,
    ui: u32,
    processors: usize,
    path: String,
    previous: Option<(Instant, Times, HashMap<u32, ThreadSample>)>,
    stages: [StageCpu; 3],
}
impl CpuProfiler {
    pub fn new() -> Self {
        Self {
            pid: std::process::id(),
            ui: unsafe { GetCurrentThreadId() },
            processors: std::thread::available_parallelism().map_or(1, usize::from),
            path: std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            previous: None,
            stages: [StageCpu::default(); 3],
        }
    }
    pub fn thread_stamp() -> Option<CpuStamp> {
        if !p2pshare_media::diagnostics::enabled() {
            return None;
        }
        times(unsafe { GetCurrentThread() }, false)
            .ok()
            .map(CpuStamp)
    }
    pub fn record_stage(&mut self, stage: UiStage, start: Option<CpuStamp>) -> Option<CpuStamp> {
        let end = Self::thread_stamp();
        if let (Some(start), Some(end)) = (&start, &end) {
            let (user, kernel) = delta(end.0, start.0);
            let totals = &mut self.stages[stage as usize];
            totals.user_ms += user;
            totals.kernel_ms += kernel;
            totals.samples += 1;
        }
        end
    }
    pub fn sample(
        &mut self,
        sharing: bool,
        watching: usize,
        preview_active: bool,
        preview_hidden: bool,
    ) {
        if !p2pshare_media::diagnostics::enabled() {
            self.previous = None;
            self.stages = [StageCpu::default(); 3];
            return;
        }
        if self
            .previous
            .as_ref()
            .is_some_and(|(at, ..)| at.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        if let Err(error) = self.capture(sharing, watching, preview_active, preview_hidden) {
            tracing::warn!(pid=self.pid,%error,"cpu_profile_unavailable");
        }
    }
    fn capture(
        &mut self,
        sharing: bool,
        watching: usize,
        preview_active: bool,
        preview_hidden: bool,
    ) -> Result<()> {
        let now = Instant::now();
        let process = times(unsafe { GetCurrentProcess() }, true)?;
        let empty = HashMap::new();
        let current = threads(
            self.pid,
            self.ui,
            self.previous.as_ref().map_or(&empty, |(_, _, t)| t),
        )?;
        if let Some((at, old, previous)) = self.previous.take() {
            let elapsed_ms = now.duration_since(at).as_secs_f64() * 1000.;
            let (user_ms, kernel_ms) = delta(process, old);
            let (cpu_pct, core_pct) = percentages(user_ms + kernel_ms, elapsed_ms, self.processors);
            let role = match (sharing, watching > 0) {
                (true, true) => "sender_and_viewer",
                (true, false) => "sender",
                (false, true) => "viewer",
                _ => "idle",
            };
            tracing::info!(pid=self.pid,exe=%self.path,role,watching,preview_active,preview_hidden,
                cpu_pct,core_pct,user_ms,kernel_ms,elapsed_ms,logical_processors=self.processors,threads=current.len(),"process_cpu");
            for (stage, totals) in ["ui_update", "ui_draw", "ui_present"]
                .into_iter()
                .zip(self.stages)
            {
                let (cpu_pct, core_pct) = percentages(
                    totals.user_ms + totals.kernel_ms,
                    elapsed_ms,
                    self.processors,
                );
                tracing::info!(
                    pid = self.pid,
                    role,
                    stage,
                    cpu_pct,
                    core_pct,
                    user_ms = totals.user_ms,
                    kernel_ms = totals.kernel_ms,
                    samples = totals.samples,
                    elapsed_ms,
                    "ui_stage_cpu"
                );
            }
            let mut busiest = Vec::new();
            for (id, sample) in &current {
                if let Some(old) = previous
                    .get(id)
                    .filter(|p| p.times.created == sample.times.created)
                {
                    let (user_ms, kernel_ms) = delta(sample.times, old.times);
                    if user_ms + kernel_ms > 0. {
                        busiest.push((*id, &sample.name, user_ms, kernel_ms));
                    }
                }
            }
            busiest.sort_by(|a, b| (b.2 + b.3).total_cmp(&(a.2 + a.3)));
            for (id, name, user_ms, kernel_ms) in busiest.into_iter().take(16) {
                let (cpu_pct, core_pct) =
                    percentages(user_ms + kernel_ms, elapsed_ms, self.processors);
                tracing::info!(pid=self.pid,thread_id=id,thread_name=%name,role,cpu_pct,core_pct,user_ms,kernel_ms,elapsed_ms,"thread_cpu");
            }
        }
        self.stages = [StageCpu::default(); 3];
        self.previous = Some((now, process, current));
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_percent_is_normalized_and_counter_reset_does_not_underflow() {
        assert_eq!(percentages(100., 1000., 8), (1.25, 10.));
        let old = Times {
            created: 1,
            kernel: 100,
            user: 100,
        };
        let new = Times {
            created: 2,
            kernel: 10,
            user: 10,
        };
        assert_eq!(delta(new, old), (0., 0.));
        assert_eq!(std::mem::size_of::<ThreadEntry>(), 28);
    }
    #[test]
    fn sampler_reads_own_named_thread_and_separates_lifetimes() {
        let mut profiler = CpuProfiler::new();
        let (ready, started) = std::sync::mpsc::channel();
        let (stop, done) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("p2pshare-profile-test".into())
            .spawn(move || {
                ready.send(unsafe { GetCurrentThreadId() }).unwrap();
                while done.try_recv().is_err() {
                    std::hint::spin_loop();
                }
            })
            .unwrap();
        let id = started.recv().unwrap();
        profiler.capture(false, 1, false, false).unwrap();
        let rows = &profiler.previous.as_ref().unwrap().2;
        assert_eq!(rows.get(&id).unwrap().name, "p2pshare-profile-test");
        assert!(rows.contains_key(&profiler.ui));
        let old = rows.get(&id).unwrap().times;
        std::thread::sleep(Duration::from_millis(200));
        profiler.capture(true, 0, false, true).unwrap();
        let current = profiler
            .previous
            .as_ref()
            .unwrap()
            .2
            .get(&id)
            .unwrap()
            .times;
        let (user, kernel) = delta(current, old);
        assert!(
            user + kernel > 0.,
            "Profiler missed CPU work on its own named thread"
        );
        stop.send(()).unwrap();
        thread.join().unwrap();
        profiler.capture(true, 0, false, true).unwrap();
        assert!(!profiler.previous.as_ref().unwrap().2.contains_key(&id));
    }
}
