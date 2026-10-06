//! Published container port attribution (witr-style)
//!
//! When a published port is bound by a container forwarder process
//! (com.docker.backend, docker-proxy, vpnkit, wslrelay, podman), the
//! interesting owner is not the forwarder but the container behind it.
//! The mapping comes from `docker ps` / `podman ps --format`, parsed for
//! HOST:PORT->TARGET segments; both engines are optional and a missing
//! or unhealthy one is a plain empty list, never an error. Nothing here
//! is a background scan — the frontend calls this once per modal session.

use serde::Serialize;

/// One published port mapped back to its container
#[derive(Serialize, Debug)]
pub struct ContainerPort {
    /// Host IP the port is published on ("0.0.0.0", "127.0.0.1", "::")
    pub host_ip: String,
    /// Published (host-side) port
    pub port: u16,
    /// Container-side port the published port forwards to
    pub target_port: u16,
    /// Container name
    pub container: String,
    /// Image the container runs
    pub image: String,
    /// Engine the entry came from ("docker" | "podman")
    pub engine: String,
}

/// docker ps format: Names|Image|Ports, ports comma-separated as
/// "0.0.0.0:8080->80/tcp" / "[::]:8080->80/tcp"
const PS_FORMAT: &str = "{{.Names}}|{{.Image}}|{{.Ports}}";

/// Forwarder processes whose ports belong to a container, not to
/// themselves — matched against the normalized process name
pub const FORWARDER_NAMES: &[&str] = &[
    "docker-proxy",
    "com.docker.backend",
    "vpnkit",
    "wslrelay",
    "podman",
];

pub fn list() -> Vec<ContainerPort> {
    let mut out = Vec::new();
    for engine in ["docker", "podman"] {
        out.extend(engine_ports(engine));
    }
    out
}

fn engine_ports(engine: &str) -> Vec<ContainerPort> {
    let Some(output) = run_ps(engine) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in output.lines() {
        let mut parts = line.split('|');
        let (Some(container), Some(image), Some(ports)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (container, image) = (container.trim(), image.trim());
        if container.is_empty() {
            continue;
        }
        for segment in ports.split(',') {
            let Some((host_side, target)) = segment.trim().split_once("->")
            else {
                continue; // exposed-only port (no publish), or noise
            };
            let Some((host_ip, port_str)) = host_side.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = port_str.parse::<u16>() else {
                continue;
            };
            let target_port = target
                .split('/')
                .next()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(0);
            out.push(ContainerPort {
                host_ip: host_ip.trim_matches(['[', ']']).to_string(),
                port,
                target_port,
                container: container.to_string(),
                image: image.to_string(),
                engine: engine.to_string(),
            });
        }
    }
    out
}

fn run_ps(engine: &str) -> Option<String> {
    let output = base_command(engine)
        .args(["ps", "--format", PS_FORMAT])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Engine command with the platform console-window suppression applied
#[cfg(windows)]
fn base_command(engine: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    // Keeps the engine CLI from flashing a console window on every query
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new(engine);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn base_command(engine: &str) -> std::process::Command {
    std::process::Command::new(engine)
}
