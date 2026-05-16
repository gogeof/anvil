//! Multi-layer security sandbox model.
//!
//! Defines a formal layer taxonomy (L0–L3) with escalating protections,
//! dynamic layer escalation/de-escalation based on command risk level,
//! and integration with the existing [`crate::sandbox`] infrastructure.
//!
//! ## Layer Model
//!
//! | Layer | Name          | Filesystem | Namespace | Network | Use Case               |
//! |-------|---------------|------------|-----------|---------|------------------------|
//! | L0    | None          | Off        | No        | No      | Trusted commands       |
//! | L1    | Filesystem    | Workspace  | No        | No      | Read-only file ops     |
//! | L2    | Standard      | Workspace  | Yes       | No      | Normal development     |
//! | L3    | Maximum       | AllowList  | Yes       | Yes     | Untrusted / external   |

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::sandbox::FilesystemIsolationMode;

/// Formal sandbox layer identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SandboxLayer {
    /// No sandboxing at all (filesystem mode = Off).
    L0,
    /// Filesystem-only isolation (workspace chroot, no namespace/network separation).
    L1,
    /// Standard isolation (workspace + namespace separation, no network isolation).
    L2,
    /// Maximum isolation (allow-list filesystem + namespace + network isolation).
    L3,
}

impl SandboxLayer {
    /// Return all layers in ascending order.
    #[must_use]
    pub fn all() -> &'static [SandboxLayer] {
        &[SandboxLayer::L0, SandboxLayer::L1, SandboxLayer::L2, SandboxLayer::L3]
    }

    /// Human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SandboxLayer::L0 => "none",
            SandboxLayer::L1 => "filesystem",
            SandboxLayer::L2 => "standard",
            SandboxLayer::L3 => "maximum",
        }
    }

    /// Corresponding [`FilesystemIsolationMode`].
    #[must_use]
    pub fn filesystem_mode(self) -> FilesystemIsolationMode {
        match self {
            SandboxLayer::L0 => FilesystemIsolationMode::Off,
            SandboxLayer::L1 | SandboxLayer::L2 => FilesystemIsolationMode::WorkspaceOnly,
            SandboxLayer::L3 => FilesystemIsolationMode::AllowList,
        }
    }

    /// Whether namespace (PID/IPC/mount) restrictions are applied.
    #[must_use]
    pub fn namespace_restricted(self) -> bool {
        matches!(self, SandboxLayer::L2 | SandboxLayer::L3)
    }

    /// Whether network isolation is applied.
    #[must_use]
    pub fn network_isolated(self) -> bool {
        matches!(self, SandboxLayer::L3)
    }

    /// Whether filesystem isolation is active at all.
    #[must_use]
    pub fn filesystem_active(self) -> bool {
        !matches!(self, SandboxLayer::L0)
    }
}

impl fmt::Display for SandboxLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L{} ({})", *self as u8, self.label())
    }
}

/// Risk classification for a command or operation.
///
/// Used by the automatic escalation engine to decide which sandbox layer
/// should be enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Trivially safe commands (e.g., `ls`, `echo`, `printf`).
    Safe,
    /// Commands that may read or write workspace files (normal dev commands).
    Normal,
    /// Commands that could affect system state (network, install, process mgmt).
    Risky,
    /// Commands that are inherently dangerous (rm -rf /, raw socket, eval loops).
    Dangerous,
}

impl RiskLevel {
    /// Minimum sandbox layer recommended for this risk level.
    #[must_use]
    pub fn recommended_layer(self) -> SandboxLayer {
        match self {
            RiskLevel::Safe => SandboxLayer::L0,
            RiskLevel::Normal => SandboxLayer::L1,
            RiskLevel::Risky => SandboxLayer::L2,
            RiskLevel::Dangerous => SandboxLayer::L3,
        }
    }
}

/// A combined layer-and-risk policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerPolicy {
    /// The base (configured) layer.
    pub base_layer: SandboxLayer,
    /// Whether automatic escalation is enabled.
    pub auto_escalate: bool,
    /// Whether automatic de-escalation is enabled.
    pub auto_deescalate: bool,
}

impl Default for LayerPolicy {
    fn default() -> Self {
        Self {
            base_layer: SandboxLayer::L2,
            auto_escalate: true,
            auto_deescalate: false,
        }
    }
}

/// Classify a shell command into a [`RiskLevel`].
///
/// This uses heuristic token scanning. It is *not* a formal security boundary;
/// the real security is enforced by the sandbox itself.
#[must_use]
pub fn classify_command_risk(command: &str) -> RiskLevel {
    let lower = command.trim().to_ascii_lowercase();

    // Empty or whitespace-only commands are safe
    if lower.is_empty() {
        return RiskLevel::Safe;
    }

    let first_token = lower.split_whitespace().next().unwrap_or("");
    let base = first_token.rsplit('/').next().unwrap_or(first_token);

    // --- Dangerous ---
    // Check prefix-based first (e.g. mkfs.ext4, chown.nfs)
    let dangerous_prefixes = [
        "mkfs", "fsck",
    ];
    for prefix in &dangerous_prefixes {
        if base.starts_with(prefix) && (base.len() == prefix.len() || base.as_bytes().get(prefix.len()) == Some(&b'.')) {
            return RiskLevel::Dangerous;
        }
    }

    let dangerous_cmds = [
        "rm", "dd", "mkfs", "fdisk", "parted", "mount", "umount",
        "chmod", "chown", "sudo", "su", "passwd", "reboot", "shutdown",
        "halt", "poweroff", "init", "systemctl", "iptables", "nft",
        "kmod", "modprobe", "insmod", "rmmod", "fsck",
    ];
    if dangerous_cmds.contains(&base) {
        return RiskLevel::Dangerous;
    }
    // rm -rf /* pattern
    if base == "rm" && (lower.contains(" -rf /") || lower.contains(" -rf /*")) {
        return RiskLevel::Dangerous;
    }

    // --- Risky ---
    let risky_cmds = [
        "curl", "wget", "nc", "nmap", "ssh", "scp", "rsync",
        "apt", "apt-get", "yum", "dnf", "pacman", "brew", "pip", "pip3",
        "npm", "yarn", "cargo install", "go install", "gem install",
        "docker", "podman", "kubectl", "helm",
        "kill", "pkill", "killall", "renice", "nice", "ulimit",
        "nslookup", "dig", "host", "ping", "traceroute", "telnet",
        "git clone", "git push",
    ];
    if risky_cmds.contains(&base) {
        return RiskLevel::Risky;
    }

    // --- Normal ---
    let normal_cmds = [
        "cat", "head", "tail", "less", "more", "wc",
        "ls", "find", "grep", "rg", "awk", "sed",
        "echo", "printf", "which", "where", "whoami", "pwd",
        "env", "printenv", "date", "cal", "df", "du",
        "free", "uptime", "uname", "file", "stat",
        "diff", "sort", "uniq", "tr", "cut", "paste",
        "tee", "xargs", "test", "true", "false", "type",
        "readlink", "realpath", "basename", "dirname",
        "sha256sum", "md5sum", "b3sum", "xxd", "hexdump", "od",
        "strings", "tree", "jq", "yq",
        "python3", "python", "node", "ruby", "perl",
        "cargo", "rustc", "rustup",
        "git", "gh",
        "make", "cmake", "meson", "ninja",
        "mkdir", "cp", "mv", "touch", "ln", "chmod",
        "tar", "gzip", "gunzip", "bzip2", "xz", "unzip", "zip",
        "vim", "nano", "emacs", "code", "nvim",
    ];
    if normal_cmds.contains(&base) {
        // Check for dangerous flags
        if lower.contains(" -i ") || lower.contains(" --in-place") {
            return RiskLevel::Risky;
        }
        return RiskLevel::Normal;
    }

    // Default: commands we can't classify are treated as Normal
    RiskLevel::Normal
}

/// Resolve the effective sandbox layer given the configured policy and command risk.
///
/// * `policy` — the configured layer policy (base layer + auto flags).
/// * `risk` — the classified risk level of the command.
/// * `user_override` — an optional explicit override from the caller.
///
/// Returns the effective [`SandboxLayer`] to apply.
#[must_use]
pub fn resolve_effective_layer(
    policy: &LayerPolicy,
    risk: RiskLevel,
    user_override: Option<SandboxLayer>,
) -> SandboxLayer {
    // User override always wins
    if let Some(layer) = user_override {
        return layer;
    }

    let recommended = risk.recommended_layer();

    if risk >= risk_for_layer(policy.base_layer) {
        // Command is at least as risky as the base layer can handle
        if policy.auto_escalate && recommended > policy.base_layer {
            // Escalate if needed
            return recommended;
        }
        policy.base_layer
    } else {
        // Command is safer than the base layer
        if policy.auto_deescalate && recommended < policy.base_layer {
            return recommended;
        }
        policy.base_layer
    }
}

/// Convert a sandbox layer back to the minimum risk level it is designed for.
#[must_use]
pub fn risk_for_layer(layer: SandboxLayer) -> RiskLevel {
    match layer {
        SandboxLayer::L0 => RiskLevel::Safe,
        SandboxLayer::L1 => RiskLevel::Normal,
        SandboxLayer::L2 => RiskLevel::Risky,
        SandboxLayer::L3 => RiskLevel::Dangerous,
    }
}

/// Configuration for the multi-layer sandbox, intended to live inside
/// [`crate::sandbox::SandboxConfig`]'s extended form or in settings.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerSandboxConfig {
    /// Explicit layer override (overrides all auto-detection).
    pub layer: Option<SandboxLayer>,
    /// Base layer policy for automatic resolution.
    pub policy: LayerPolicy,
    /// Additional allowed mounts for L3.
    pub allowed_mounts: Vec<String>,
}

impl Default for LayerSandboxConfig {
    fn default() -> Self {
        Self {
            layer: None,
            policy: LayerPolicy::default(),
            allowed_mounts: Vec::new(),
        }
    }
}

// ============================================================================
// Integration helpers
// ============================================================================

/// Map from [`SandboxLayer`] to the existing sandbox boolean flags.
///
/// This bridges the layer model with [`crate::sandbox::SandboxRequest`].
pub struct LayerSandboxFlags {
    pub enabled: bool,
    pub namespace_restrictions: bool,
    pub network_isolation: bool,
    pub filesystem_mode: FilesystemIsolationMode,
}

impl From<SandboxLayer> for LayerSandboxFlags {
    fn from(layer: SandboxLayer) -> Self {
        Self {
            enabled: layer.filesystem_active(),
            namespace_restrictions: layer.namespace_restricted(),
            network_isolation: layer.network_isolated(),
            filesystem_mode: layer.filesystem_mode(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Layer properties ---

    #[test]
    fn l0_disables_everything() {
        let flags = LayerSandboxFlags::from(SandboxLayer::L0);
        assert!(!flags.enabled);
        assert!(!flags.namespace_restrictions);
        assert!(!flags.network_isolation);
        assert_eq!(flags.filesystem_mode, FilesystemIsolationMode::Off);
    }

    #[test]
    fn l1_enables_filesystem_only() {
        let flags = LayerSandboxFlags::from(SandboxLayer::L1);
        assert!(flags.enabled);
        assert!(!flags.namespace_restrictions);
        assert!(!flags.network_isolation);
        assert_eq!(flags.filesystem_mode, FilesystemIsolationMode::WorkspaceOnly);
    }

    #[test]
    fn l2_standard_isolation() {
        let flags = LayerSandboxFlags::from(SandboxLayer::L2);
        assert!(flags.enabled);
        assert!(flags.namespace_restrictions);
        assert!(!flags.network_isolation);
        assert_eq!(flags.filesystem_mode, FilesystemIsolationMode::WorkspaceOnly);
    }

    #[test]
    fn l3_maximum_isolation() {
        let flags = LayerSandboxFlags::from(SandboxLayer::L3);
        assert!(flags.enabled);
        assert!(flags.namespace_restrictions);
        assert!(flags.network_isolation);
        assert_eq!(flags.filesystem_mode, FilesystemIsolationMode::AllowList);
    }

    #[test]
    fn layer_ordering() {
        assert!(SandboxLayer::L0 < SandboxLayer::L1);
        assert!(SandboxLayer::L1 < SandboxLayer::L2);
        assert!(SandboxLayer::L2 < SandboxLayer::L3);
    }

    // --- Command risk classification ---

    #[test]
    fn classifies_safe_commands() {
        assert_eq!(classify_command_risk(""), RiskLevel::Safe);
        assert_eq!(classify_command_risk("   "), RiskLevel::Safe);
    }

    #[test]
    fn classifies_normal_commands() {
        assert_eq!(classify_command_risk("ls -la"), RiskLevel::Normal);
        assert_eq!(classify_command_risk("cat /etc/passwd"), RiskLevel::Normal);
        assert_eq!(classify_command_risk("git status"), RiskLevel::Normal);
        assert_eq!(classify_command_risk("python3 script.py"), RiskLevel::Normal);
    }

    #[test]
    fn classifies_risky_commands() {
        assert_eq!(classify_command_risk("curl https://example.com"), RiskLevel::Risky);
        assert_eq!(classify_command_risk("ssh user@host"), RiskLevel::Risky);
        assert_eq!(classify_command_risk("apt-get update"), RiskLevel::Risky);
        assert_eq!(classify_command_risk("docker ps"), RiskLevel::Risky);
    }

    #[test]
    fn classifies_dangerous_commands() {
        assert_eq!(classify_command_risk("rm -rf /"), RiskLevel::Dangerous);
        assert_eq!(classify_command_risk("dd if=/dev/zero of=/dev/sda"), RiskLevel::Dangerous);
        assert_eq!(classify_command_risk("sudo !!"), RiskLevel::Dangerous);
        assert_eq!(classify_command_risk("mkfs.ext4 /dev/sdb1"), RiskLevel::Dangerous);
    }

    #[test]
    fn sed_in_place_is_risky() {
        assert_eq!(classify_command_risk("sed -i 's/a/b/' file"), RiskLevel::Risky);
    }

    #[test]
    fn full_path_commands_classified() {
        assert_eq!(classify_command_risk("/usr/bin/cat file"), RiskLevel::Normal);
        assert_eq!(classify_command_risk("/usr/bin/curl url"), RiskLevel::Risky);
    }

    // --- Layer resolution ---

    #[test]
    fn base_layer_used_when_risk_matches() {
        let policy = LayerPolicy {
            base_layer: SandboxLayer::L2,
            ..Default::default()
        };
        // Normal command → recommended L1, but base is L2, so L2
        assert_eq!(
            resolve_effective_layer(&policy, RiskLevel::Normal, None),
            SandboxLayer::L2
        );
    }

    #[test]
    fn auto_escalates_to_handle_risk() {
        let policy = LayerPolicy {
            base_layer: SandboxLayer::L1,
            auto_escalate: true,
            auto_deescalate: false,
        };
        // Dangerous command → recommended L3, escalates
        assert_eq!(
            resolve_effective_layer(&policy, RiskLevel::Dangerous, None),
            SandboxLayer::L3
        );
    }

    #[test]
    fn auto_escalation_can_be_disabled() {
        let policy = LayerPolicy {
            base_layer: SandboxLayer::L1,
            auto_escalate: false,
            ..Default::default()
        };
        // Dangerous command but no escalation → stays L1
        assert_eq!(
            resolve_effective_layer(&policy, RiskLevel::Dangerous, None),
            SandboxLayer::L1
        );
    }

    #[test]
    fn user_override_takes_precedence() {
        let policy = LayerPolicy {
            base_layer: SandboxLayer::L2,
            ..Default::default()
        };
        assert_eq!(
            resolve_effective_layer(&policy, RiskLevel::Dangerous, Some(SandboxLayer::L0)),
            SandboxLayer::L0
        );
    }

    #[test]
    fn auto_deescalates_safe_commands() {
        let policy = LayerPolicy {
            base_layer: SandboxLayer::L2,
            auto_escalate: true,
            auto_deescalate: true,
        };
        // Safe command with de-escalation → drops to L0
        assert_eq!(
            resolve_effective_layer(&policy, RiskLevel::Safe, None),
            SandboxLayer::L0
        );
    }

    #[test]
    fn deescalation_requires_opt_in() {
        let policy = LayerPolicy {
            base_layer: SandboxLayer::L2,
            auto_escalate: true,
            auto_deescalate: false,
        };
        // Safe command but no de-escalation → stays L2
        assert_eq!(
            resolve_effective_layer(&policy, RiskLevel::Safe, None),
            SandboxLayer::L2
        );
    }

    #[test]
    fn layer_labels() {
        assert_eq!(SandboxLayer::L0.label(), "none");
        assert_eq!(SandboxLayer::L1.label(), "filesystem");
        assert_eq!(SandboxLayer::L2.label(), "standard");
        assert_eq!(SandboxLayer::L3.label(), "maximum");
    }

    #[test]
    fn risk_level_ordering() {
        assert!(RiskLevel::Safe < RiskLevel::Normal);
        assert!(RiskLevel::Normal < RiskLevel::Risky);
        assert!(RiskLevel::Risky < RiskLevel::Dangerous);
    }

    #[test]
    fn recommended_layer_for_each_risk() {
        assert_eq!(RiskLevel::Safe.recommended_layer(), SandboxLayer::L0);
        assert_eq!(RiskLevel::Normal.recommended_layer(), SandboxLayer::L1);
        assert_eq!(RiskLevel::Risky.recommended_layer(), SandboxLayer::L2);
        assert_eq!(RiskLevel::Dangerous.recommended_layer(), SandboxLayer::L3);
    }
}
