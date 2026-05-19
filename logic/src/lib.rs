// no_std only when compiled for embedded (bare-metal) targets.
#![cfg_attr(target_os = "none", no_std)]

pub mod debouncer;
pub mod led_dispatch;
pub mod config;
