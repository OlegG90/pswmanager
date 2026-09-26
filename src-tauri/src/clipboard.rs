//! Copying secrets: kept out of clipboard history and cloud clipboard, and
//! cleared after a while unless something else has been copied since.

use arboard::SetExtWindows;
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
use zeroize::Zeroizing;

/// The clipboard's sequence number right after our last copy.
static OURS: Mutex<Option<u32>> = Mutex::new(None);

fn sequence() -> u32 {
    // SAFETY: no arguments, no preconditions.
    unsafe { GetClipboardSequenceNumber() }
}

/// Puts `text` on the clipboard and clears it after `clear_after`.
pub fn copy(text: Zeroizing<String>, clear_after: Duration) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let mut ours = OURS.lock().unwrap();
    clipboard
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .exclude_from_monitoring()
        .text(text.as_str())
        .map_err(|e| e.to_string())?;
    let copied = sequence();
    *ours = Some(copied);
    drop(ours);
    std::thread::spawn(move || {
        std::thread::sleep(clear_after);
        clear_if(copied);
    });
    Ok(())
}

/// Clears the clipboard if it still holds what we copied last (on lock and quit).
pub fn clear_if_ours() {
    let copied = *OURS.lock().unwrap();
    if let Some(copied) = copied {
        clear_if(copied);
    }
}

fn clear_if(copied: u32) {
    let mut ours = OURS.lock().unwrap();
    if *ours == Some(copied) && sequence() == copied {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.clear();
        }
        *ours = None;
    }
}
