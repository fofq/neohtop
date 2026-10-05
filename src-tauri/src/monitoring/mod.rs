//! System monitoring functionality
//!
//! This module provides types and functionality for monitoring system resources
//! and processes. It includes process monitoring, system statistics collection,
//! and data structures for representing system state.

pub(crate) mod network_ports;
mod process_monitor;
mod system_monitor;
mod types;
// Shared grow-and-retry helpers for the Win32 buffer enumeration commands
// below; windows-only because every user is
#[cfg(windows)]
mod winbuf;
// Exposed as a module path because its function names collide with the
// Tauri commands in commands.rs (e.g. restart_as_admin)
pub(crate) mod process_control;
// These are exposed as module paths (like process_control above) so the
// Tauri commands in commands.rs call them unambiguously
pub(crate) mod file_lockers;
pub(crate) mod process_inspection;
pub(crate) mod sid_name;
pub(crate) mod startup_items;
pub(crate) mod services;
pub(crate) mod tcp_control;
pub(crate) mod window_list;

pub use file_lockers::FileLocker;
pub use process_inspection::{DriverInfo, ModuleInfo, ProcessMetadata};
pub use services::ServiceInfo;
pub use window_list::AppWindow;
pub use network_ports::{
    PortConnection, TrafficCounters, collect as collect_network_ports, listening_ports,
    traffic_counters,
};
pub use process_control::ProcessPriorityInfo;
pub use process_control::KillTreeResult;
pub use startup_items::StartupItem;
pub use process_monitor::ProcessMonitor;
pub use system_monitor::SystemMonitor;
pub use types::*; // Re-export all types
