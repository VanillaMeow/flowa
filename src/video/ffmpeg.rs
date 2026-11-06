//! Bindings generated from the FFmpeg SDK linked into this build

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals
)]
#![allow(clippy::all)]
#![allow(unnecessary_transmutes)]

include!(concat!(env!("OUT_DIR"), "/ffmpeg.rs"));
