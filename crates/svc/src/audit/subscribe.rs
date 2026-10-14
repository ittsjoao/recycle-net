//! EvtSubscribe on the Security channel: keep 4663 deletes under protected folders.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::nests;
use crate::index::Index;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::EventLog::{
    EvtClose, EvtCreateBookmark, EvtRender, EvtRenderBookmark, EvtRenderEventXml, EvtSubscribe,
    EvtSubscribeActionDeliver, EvtSubscribeStartAfterBookmark, EvtSubscribeToFutureEvents, EvtUpdateBookmark,
    EVT_HANDLE, EVT_RENDER_FLAGS, EVT_SUBSCRIBE_NOTIFY_ACTION,
};

const QUERY: &str = "*[System[EventID=4663]]";
const BOOKMARK_KEY: &str = "audit.bookmark";
const SAVE_BOOKMARK_EVERY: Duration = Duration::from_secs(30);

/// The path lies inside one of the protected folders.
pub fn under_any(roots: &[String], path: &str) -> bool {
    let p = std::path::Path::new(path);
    roots.iter().any(|r| nests(std::path::Path::new(r), p))
}

struct Sink {
    tx: mpsc::Sender<String>,
    bookmark: Mutex<EVT_HANDLE>,
}

fn render(h: EVT_HANDLE, flags: EVT_RENDER_FLAGS) -> Option<String> {
    unsafe {
        let (mut used, mut props) = (0u32, 0u32);
        let _ = EvtRender(None, h, flags.0, 0, None, &mut used, &mut props);
        if used == 0 {
            return None;
        }
        let mut buf = vec![0u16; (used as usize).div_ceil(2)];
        EvtRender(None, h, flags.0, (buf.len() * 2) as u32, Some(buf.as_mut_ptr().cast()), &mut used, &mut props).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

unsafe extern "system" fn on_event(
    action: EVT_SUBSCRIBE_NOTIFY_ACTION,
    ctx: *const core::ffi::c_void,
    ev: EVT_HANDLE,
) -> u32 {
    let sink = &*(ctx as *const Sink);
    if action == EvtSubscribeActionDeliver {
        if let Some(xml) = render(ev, EvtRenderEventXml) {
            let _ = sink.tx.send(xml);
        }
        if let Ok(bm) = sink.bookmark.lock() {
            let _ = EvtUpdateBookmark(*bm, ev);
        }
    }
    0
}

fn save_bookmark(idx: &Index, sink: &Sink) {
    if let Ok(bm) = sink.bookmark.lock() {
        if let Some(xml) = render(*bm, EvtRenderBookmark) {
            if let Err(e) = idx.set_kv(BOOKMARK_KEY, &xml) {
                tracing::warn!("audit: cannot save bookmark: {e}");
            }
        }
    }
}

/// Runs until `stop`. Resumes after the saved bookmark, so a short service
/// restart loses no event still in the log.
pub fn run(idx: Arc<Index>, roots: Vec<String>, stop: Arc<AtomicBool>) -> Result<(), String> {
    let saved = idx.get_kv(BOOKMARK_KEY).map_err(|e| e.to_string())?;
    let bookmark = unsafe {
        match &saved {
            Some(xml) => EvtCreateBookmark(&HSTRING::from(xml.as_str())),
            None => EvtCreateBookmark(PCWSTR::null()),
        }
    }
    .map_err(|e| format!("EvtCreateBookmark: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let sink = Box::new(Sink { tx, bookmark: Mutex::new(bookmark) });
    let (start, flags) = match saved {
        Some(_) => (bookmark, EvtSubscribeStartAfterBookmark.0),
        None => (EVT_HANDLE::default(), EvtSubscribeToFutureEvents.0),
    };
    let sub = unsafe {
        EvtSubscribe(
            None,
            None,
            &HSTRING::from("Security"),
            &HSTRING::from(QUERY),
            start,
            Some(&*sink as *const Sink as *const core::ffi::c_void),
            Some(on_event),
            flags,
        )
    }
    .map_err(|e| format!("EvtSubscribe: {e}"))?;
    tracing::info!("audit: subscribed to Security 4663");

    let mut last_save = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(xml) => {
                if let Some(e) = crate::audit::event::parse_4663(&xml) {
                    if under_any(&roots, &e.path) {
                        if let Err(err) = idx.add_delete_event(&e) {
                            tracing::warn!("audit: cannot store event: {err}");
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_save.elapsed() >= SAVE_BOOKMARK_EVERY {
            save_bookmark(&idx, &sink);
            last_save = Instant::now();
        }
    }
    // Close the subscription first: no callback may run after `sink` is dropped.
    unsafe {
        let _ = EvtClose(sub);
    }
    save_bookmark(&idx, &sink);
    unsafe {
        let _ = EvtClose(bookmark);
    }
    drop(sink);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_any_is_case_insensitive_and_needs_a_real_parent() {
        let roots = vec![r"D:\Dados".to_string(), r"E:\".to_string()];
        assert!(under_any(&roots, r"d:\dados\x\a.txt"));
        assert!(under_any(&roots, r"E:\a.txt"));
        assert!(!under_any(&roots, r"D:\DadosOld\a.txt"));
        assert!(!under_any(&roots, r"C:\Windows\Temp\a.tmp"));
    }
}
