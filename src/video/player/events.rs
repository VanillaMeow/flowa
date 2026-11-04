//! Applies typed libmpv events before the UI reads playback state

use std::ffi::CStr;

use super::{Player, VideoState, mpv};

const MAX_EVENTS_PER_POLL: usize = 128;

impl Player {
    pub fn poll(&mut self, state: &mut VideoState) {
        if self.swap_pending {
            // SAFETY: The preceding egui frame has been submitted before the next UI update
            unsafe { (self.api.render_context_report_swap)(self.render) };
            self.swap_pending = false;
        }

        // Advanced control requires servicing GPU requests before FILE_LOADED can arrive
        self.update_render_context();

        for _ in 0..MAX_EVENTS_PER_POLL {
            // SAFETY: Events and their payloads are read before the next wait_event call
            let event = unsafe { &*(self.api.wait_event)(self.handle, 0.) };
            match event.id {
                mpv::EVENT_NONE => break,
                mpv::EVENT_FILE_LOADED => state.loading = false,
                mpv::EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                    let property = unsafe { &*event.data.cast::<mpv::Property>() };
                    apply_property(state, property);
                }
                mpv::EVENT_END_FILE if !event.data.is_null() => {
                    let end = unsafe { &*event.data.cast::<mpv::EndFile>() };
                    if end.reason == mpv::END_FILE_ERROR {
                        state.loading = false;
                        state.error = Some(
                            self.api
                                .check(end.error)
                                .err()
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "Video playback failed".into()),
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

fn apply_property(state: &mut VideoState, property: &mpv::Property) {
    if property.data.is_null() {
        return;
    }

    // SAFETY: Libmpv supplies a live property payload of the observed type
    let name = unsafe { CStr::from_ptr(property.name) }.to_bytes();
    unsafe {
        match (name, property.format) {
            (b"pause", mpv::FORMAT_FLAG) => {
                state.paused = *property.data.cast::<i32>() != 0
            }
            (b"time-pos", mpv::FORMAT_DOUBLE) => {
                state.position = *property.data.cast::<f64>()
            }
            (b"duration", mpv::FORMAT_DOUBLE) => {
                state.duration = *property.data.cast::<f64>()
            }
            (b"volume", mpv::FORMAT_DOUBLE) => {
                state.volume = *property.data.cast::<f64>()
            }
            (b"video-params/dw", mpv::FORMAT_INT64) => {
                state.size[0] =
                    (*property.data.cast::<i64>()).clamp(0, u32::MAX as i64) as u32
            }
            (b"video-params/dh", mpv::FORMAT_INT64) => {
                state.size[1] =
                    (*property.data.cast::<i64>()).clamp(0, u32::MAX as i64) as u32
            }
            (b"hwdec-current", mpv::FORMAT_STRING) => {
                let value = *property.data.cast::<*const std::ffi::c_char>();
                if !value.is_null() {
                    state.decoder =
                        CStr::from_ptr(value).to_string_lossy().into_owned();
                    log::info!("Video decoder: {}", state.decoder);
                }
            }
            (b"current-ao", mpv::FORMAT_STRING) => {
                let value = *property.data.cast::<*const std::ffi::c_char>();

                if !value.is_null() {
                    log::info!(
                        "Audio output: {}",
                        CStr::from_ptr(value).to_string_lossy()
                    );
                }
            }
            _ => {}
        }
    }
}
