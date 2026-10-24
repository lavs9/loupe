//! In-app media player for audio (mp3, m4a, flac, wav, aac) and video (mp4, mov).
//!
//! Uses macOS AVFoundation to play the original files on disk without transcoding.

#![allow(unexpected_cfgs)]

use std::path::{Path, PathBuf};

#[cfg(all(feature = "gui", target_os = "macos"))]
use objc::runtime::Object;
#[cfg(all(feature = "gui", target_os = "macos"))]
use objc::{class, msg_send, sel, sel_impl};

#[cfg(all(feature = "gui", target_os = "macos"))]
#[link(name = "AVFoundation", kind = "framework")]
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl CMTime {
    pub fn from_seconds(seconds: f64) -> Self {
        if seconds <= 0.0 || seconds.is_nan() {
            return Self {
                value: 0,
                timescale: 1000,
                flags: 1,
                epoch: 0,
            };
        }
        Self {
            value: (seconds * 1000.0) as i64,
            timescale: 1000,
            flags: 1,
            epoch: 0,
        }
    }

    pub fn to_seconds(self) -> Option<f64> {
        // flags: 1 = kCMTimeFlags_Valid
        if (self.flags & 1) != 0 && self.timescale > 0 {
            Some(self.value as f64 / self.timescale as f64)
        } else {
            None
        }
    }
}

pub struct MediaPlayer {
    #[cfg(all(feature = "gui", target_os = "macos"))]
    player: *mut Object,
    current_path: Option<PathBuf>,
    cached_duration: Option<f64>,
}

unsafe impl Send for MediaPlayer {}
unsafe impl Sync for MediaPlayer {}

impl MediaPlayer {
    pub fn new() -> Self {
        Self {
            #[cfg(all(feature = "gui", target_os = "macos"))]
            player: std::ptr::null_mut(),
            current_path: None,
            cached_duration: None,
        }
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.current_path.as_deref()
    }

    pub fn is_loaded(&self) -> bool {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        {
            !self.player.is_null()
        }
        #[cfg(not(all(feature = "gui", target_os = "macos")))]
        {
            self.current_path.is_some()
        }
    }

    pub fn load(&mut self, path: &Path) -> bool {
        if self.current_path.as_deref() == Some(path) && self.is_loaded() {
            return true;
        }

        self.stop();

        if !path.is_file() {
            return false;
        }

        self.current_path = Some(path.to_path_buf());
        self.cached_duration = query_file_duration(path);

        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            let path_str = match std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
                Ok(s) => s,
                Err(_) => return false,
            };

            let cls_string = class!(NSString);
            let ns_path: *mut Object = msg_send![cls_string, stringWithUTF8String: path_str.as_ptr()];
            if ns_path.is_null() {
                return false;
            }

            let cls_url = class!(NSURL);
            let ns_url: *mut Object = msg_send![cls_url, fileURLWithPath: ns_path];
            if ns_url.is_null() {
                return false;
            }

            let cls_player = class!(AVPlayer);
            let player: *mut Object = msg_send![cls_player, playerWithURL: ns_url];
            if player.is_null() {
                return false;
            }

            let _: () = msg_send![player, retain];
            self.player = player;
            true
        }

        #[cfg(not(all(feature = "gui", target_os = "macos")))]
        {
            true
        }
    }

    pub fn play(&mut self) {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if !self.player.is_null() {
                // If at or near the end, restart from 0
                if let (Some(cur), Some(dur)) = (self.current_time(), self.duration()) {
                    if dur > 0.0 && cur >= dur - 0.15 {
                        self.seek(0.0);
                    }
                }
                let _: () = msg_send![self.player, play];
            }
        }
    }

    pub fn pause(&mut self) {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if !self.player.is_null() {
                let _: () = msg_send![self.player, pause];
            }
        }
    }

    pub fn toggle_play_pause(&mut self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn is_playing(&self) -> bool {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if self.player.is_null() {
                return false;
            }
            // timeControlStatus: 0 = paused, 1 = waitingToPlay, 2 = playing
            let status: i64 = msg_send![self.player, timeControlStatus];
            status == 2
        }
        #[cfg(not(all(feature = "gui", target_os = "macos")))]
        {
            false
        }
    }

    pub fn current_time(&self) -> Option<f64> {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if self.player.is_null() {
                return None;
            }
            let time: CMTime = msg_send![self.player, currentTime];
            time.to_seconds()
        }
        #[cfg(not(all(feature = "gui", target_os = "macos")))]
        {
            None
        }
    }

    pub fn duration(&self) -> Option<f64> {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if !self.player.is_null() {
                let item: *mut Object = msg_send![self.player, currentItem];
                if !item.is_null() {
                    let dur: CMTime = msg_send![item, duration];
                    if let Some(secs) = dur.to_seconds() {
                        if secs > 0.0 && !secs.is_nan() && !secs.is_infinite() {
                            return Some(secs);
                        }
                    }
                }
            }
        }
        self.cached_duration
    }

    #[allow(unused_variables)]
    pub fn seek(&mut self, seconds: f64) {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if !self.player.is_null() {
                let target = CMTime::from_seconds(seconds);
                let _: () = msg_send![self.player, seekToTime: target];
            }
        }
    }

    pub fn stop(&mut self) {
        #[cfg(all(feature = "gui", target_os = "macos"))]
        unsafe {
            if !self.player.is_null() {
                let _: () = msg_send![self.player, pause];
                let _: () = msg_send![self.player, release];
                self.player = std::ptr::null_mut();
            }
        }
        self.current_path = None;
        self.cached_duration = None;
    }
}

impl Drop for MediaPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Query duration in seconds via macOS Spotlight metadata (mdls).
pub fn query_file_duration(path: &Path) -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/mdls")
            .args(["-raw", "-name", "kMDItemDurationSeconds"])
            .arg(path)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() || text == "(null)" {
            return None;
        }
        text.parse::<f64>().ok().filter(|d| *d > 0.0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Format seconds as MM:SS (or H:MM:SS if >= 1 hour).
pub fn format_duration(seconds: f64) -> String {
    if seconds.is_nan() || seconds.is_infinite() || seconds < 0.0 {
        return "0:00".to_string();
    }
    let total_secs = seconds.round() as u64;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0.0), "0:00");
        assert_eq!(format_duration(45.2), "0:45");
        assert_eq!(format_duration(65.0), "1:05");
        assert_eq!(format_duration(3605.0), "1:00:05");
        assert_eq!(format_duration(-5.0), "0:00");
    }

    #[test]
    fn test_cmtime_conversion() {
        let t = CMTime::from_seconds(12.5);
        assert_eq!(t.value, 12500);
        assert_eq!(t.timescale, 1000);
        assert!((t.to_seconds().unwrap() - 12.5).abs() < 1e-4);
    }

    #[test]
    fn test_documents_playable_mime() {
        use crate::documents::*;
        assert!(is_audio("audio/mpeg"));
        assert!(is_audio("audio/mp4"));
        assert!(is_audio("audio/flac"));
        assert!(!is_audio("image/jpeg"));
        assert!(!is_audio("video/mp4"));

        assert!(is_video("video/mp4"));
        assert!(is_video("video/quicktime"));
        assert!(!is_video("audio/mpeg"));
        assert!(!is_video("image/png"));

        assert!(is_playable("audio/mpeg"));
        assert!(is_playable("video/mp4"));
        assert!(!is_playable("image/jpeg"));
    }

    #[test]
    fn test_missing_file_metadata_safe() {
        let missing = Path::new("/tmp/nonexistent-track-file-12345.mp3");
        assert_eq!(query_file_duration(missing), None);
        assert_eq!(crate::documents::audio_album_art(missing), None);
        assert_eq!(crate::documents::audio_tag_info(missing), None);
    }

    #[test]
    fn test_media_player_playback_real_wav() {
        let temp_dir = std::env::temp_dir().join("loupe-test-player");
        let _ = std::fs::create_dir_all(&temp_dir);
        let wav_path = temp_dir.join("test_tone.wav");

        // Write a valid 1-second 44.1kHz mono WAV
        let sample_rate = 44100u32;
        let num_samples = sample_rate; // 1 second
        let data_size = num_samples * 2;
        let mut bytes = Vec::with_capacity(44 + data_size as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // Mono
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for _ in 0..num_samples {
            bytes.extend_from_slice(&0i16.to_le_bytes());
        }
        std::fs::write(&wav_path, &bytes).expect("write wav");

        let mut player = MediaPlayer::new();
        assert!(player.load(&wav_path));
        assert!(player.is_loaded());
        assert_eq!(player.current_path(), Some(wav_path.as_path()));

        player.play();
        player.pause();
        assert!(!player.is_playing());

        player.seek(0.5);
        player.stop();
        assert!(!player.is_loaded());

        let _ = std::fs::remove_file(&wav_path);
    }
}
