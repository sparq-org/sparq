// sq-tg6b — the LAN/WAN network profiles for the MPC benchmark
// network tier (Tier 3). New module: the tc/netem shaping PARAMETERS as data, +
// the `tc qdisc` command-line generator that `scripts/mpc-netem.sh` applies.
//
// PRIVILEGE GATE (load-bearing honesty): applying a netem qdisc needs
// CAP_NET_ADMIN (root / sudo `tc qdisc add`). The dev box is unprivileged, so the
// LAN/WAN-SHAPED runs are NOT runnable here — only the unshaped loopback transport
// (Tier 2, `crate::transport`) is. This module therefore ships the PROFILES + the
// command generator + the apply-script (documented), gated to a privileged host or
// the EC2 tier (bead sq-hoaj). It NEVER fakes a shaped wall-clock: a Tier-3 number
// only exists when `tc` actually ran. The profile parameters are the canonical
// MPC-DB values from the literature (design record §5.3 — MP-SPDZ / Secrecy / ORQ).
//! Linux `tc`/`netem` LAN/WAN network profiles for the MPC network tier (Tier 3).
//!
//! ## What this is (and what it deliberately is not)
//!
//! A [`NetemProfile`] is the latency / bandwidth / jitter / loss parameters of one
//! emulated network condition, plus the `tc qdisc` command lines that realise it
//! on the loopback (or a veth) interface. The canonical literature profiles
//! (design record §5.3):
//!
//! PR #96 — `delay_ms` is the netem **one-way** delay and
//! [`NetemProfile::rtt_ms`] returns `2 × delay_ms`, so the labels/docs below
//! state the emulated **RTT** consistently with that (and with the `rtt_ms()`
//! tests and `scripts/mpc-netem.sh`):
//!
//! - **LAN:** 1 Gbit/s, 2 ms RTT (1 ms each way — `delay_ms = 1`), negligible
//!   jitter/loss — the datacenter / same-rack condition MP-SPDZ and Secrecy use.
//! - **WAN:** 100 Mbit/s, 100 ms RTT (50 ms each way — `delay_ms = 50`), small
//!   jitter + 0.1 % loss — the cross-region condition; ORQ reports a 1.2–6.9×
//!   WAN-over-LAN slowdown for the same workload, which the network tier emits as
//!   a ratio once both run.
//!
//! ## The privilege gate
//!
//! `tc qdisc add` requires `CAP_NET_ADMIN` (root). On an unprivileged box (this
//! one) the profiles are inert data: [`NetemProfile::apply_commands`] emits the
//! exact `tc` invocations, but running them needs sudo. So Tier 3 runs on a
//! privileged host or the orphan-proof EC2 bench (sq-hoaj) via
//! `scripts/mpc-netem.sh`; this module is the single source of truth for WHAT to
//! apply. A shaped wall-clock that was not actually produced under `tc` is never
//! reported — the harness checks the qdisc is live before trusting Tier-3 timing.

use std::fmt;

/// One emulated network condition: the netem shaping parameters + a stable id.
///
/// Units are fixed at construction (kbit for bandwidth, ms for delay) so the
/// `tc` command generation is unambiguous. All fields are public so a privileged
/// runner can tweak a profile without going through a builder.
///
/// (No `Eq` — `loss_pct` is `f64`; profiles are compared structurally only in
/// tests, where `PartialEq` is sufficient.)
#[derive(Debug, Clone, PartialEq)]
pub struct NetemProfile {
    /// Stable identifier for the JSON schema / table (`"lan"`, `"wan"`).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// One-way delay in milliseconds (RTT = `2 × delay_ms`). netem applies delay
    /// per-direction, so the full RTT is twice this when both endpoints are
    /// shaped (or this is applied to a single shared loopback).
    pub delay_ms: u32,
    /// Delay jitter in milliseconds (netem `delay <d>ms <j>ms`). 0 = constant.
    pub jitter_ms: u32,
    /// Egress bandwidth cap in kbit/s (netem `rate`). 0 = unshaped (no cap).
    pub rate_kbit: u64,
    /// Packet-loss percentage (netem `loss`), e.g. `0.1` = 0.1 %. 0 = lossless.
    pub loss_pct: f64,
}

impl NetemProfile {
    /// **LAN** — 1 Gbit/s, 2 ms RTT (1 ms one-way), no jitter/loss. The
    /// datacenter / same-rack condition (design record §5.3; MP-SPDZ / Secrecy
    /// LAN setup). PR #96: label states the emulated RTT (`2 ×
    /// delay_ms`), not the one-way `delay_ms`, so it matches [`Self::rtt_ms`].
    pub const fn lan() -> Self {
        NetemProfile {
            id: "lan",
            label: "LAN (1 Gbit/s, 2 ms RTT)",
            delay_ms: 1, // 1 ms one-way ⇒ 2 ms RTT (see rtt_ms())
            jitter_ms: 0,
            rate_kbit: 1_000_000, // 1 Gbit/s
            loss_pct: 0.0,
        }
    }

    /// **WAN** — 100 Mbit/s, 100 ms RTT (50 ms one-way), 5 ms jitter, 0.1 % loss.
    /// The cross-region federation condition (design record §5.3; the 100–200
    /// Mbit/s, 20–100 ms band the cited systems use; ORQ's WAN setup).
    /// PR #96: label states the emulated RTT (`2 × delay_ms`), not the one-way
    /// `delay_ms`, so it matches [`Self::rtt_ms`].
    pub const fn wan() -> Self {
        NetemProfile {
            id: "wan",
            label: "WAN (100 Mbit/s, 100 ms RTT, 0.1% loss)",
            delay_ms: 50, // 50 ms one-way ⇒ 100 ms RTT (see rtt_ms())
            jitter_ms: 5,
            rate_kbit: 100_000, // 100 Mbit/s
            loss_pct: 0.1,
        }
    }

    /// The unshaped baseline (Tier 2: raw loopback). Useful so the LAN→WAN
    /// multiplier has a zero-shaping anchor and the same code path runs unshaped.
    pub const fn loopback() -> Self {
        NetemProfile {
            id: "loopback",
            label: "loopback (unshaped, ~0 RTT)",
            delay_ms: 0,
            jitter_ms: 0,
            rate_kbit: 0,
            loss_pct: 0.0,
        }
    }

    /// The canonical set the network tier sweeps: unshaped loopback (Tier 2) plus
    /// the two literature-standard shaped profiles (Tier 3).
    pub fn standard_set() -> [NetemProfile; 3] {
        [Self::loopback(), Self::lan(), Self::wan()]
    }

    /// Round-trip time this profile emulates, in milliseconds (`2 × delay_ms`).
    pub fn rtt_ms(&self) -> u32 {
        self.delay_ms * 2
    }

    /// `true` if this profile applies NO shaping (the raw loopback baseline) — it
    /// needs no `tc` and therefore no privilege.
    pub fn is_unshaped(&self) -> bool {
        self.delay_ms == 0 && self.jitter_ms == 0 && self.rate_kbit == 0 && self.loss_pct == 0.0
    }

    /// The `tc qdisc` argument string (after `tc qdisc add dev <iface> root
    /// netem`) that realises this profile. Empty for the unshaped baseline.
    ///
    /// Example (WAN): `delay 50ms 5ms rate 100000kbit loss 0.1%`. This is the
    /// exact netem stanza `scripts/mpc-netem.sh` splices into the `tc` call; the
    /// generator lives here so the parameters have one home (no duplicated magic
    /// numbers in the shell script).
    pub fn netem_args(&self) -> String {
        if self.is_unshaped() {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.delay_ms > 0 {
            if self.jitter_ms > 0 {
                parts.push(format!("delay {}ms {}ms", self.delay_ms, self.jitter_ms));
            } else {
                parts.push(format!("delay {}ms", self.delay_ms));
            }
        }
        if self.rate_kbit > 0 {
            parts.push(format!("rate {}kbit", self.rate_kbit));
        }
        if self.loss_pct > 0.0 {
            // Trim trailing zeros for a clean `loss 0.1%`.
            parts.push(format!("loss {}%", trim_pct(self.loss_pct)));
        }
        parts.join(" ")
    }

    /// The full `tc` apply / clear command pair for `iface` (default loopback
    /// `lo`). The caller (`scripts/mpc-netem.sh`, run under sudo) executes these.
    /// Returns `(apply, clear)`; for the unshaped baseline `apply` is empty.
    pub fn apply_commands(&self, iface: &str) -> (String, String) {
        let clear = format!("tc qdisc del dev {iface} root 2>/dev/null || true");
        if self.is_unshaped() {
            return (String::new(), clear);
        }
        let apply = format!("tc qdisc add dev {iface} root netem {}", self.netem_args());
        (apply, clear)
    }
}

impl fmt::Display for NetemProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]", self.label, self.id)
    }
}

/// Render `pct` without trailing zeros (`0.1` not `0.100000`), for the netem
/// `loss <x>%` token.
fn trim_pct(pct: f64) -> String {
    let s = format!("{pct:.4}");
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_profile_is_gigabit_one_ms() {
        let p = NetemProfile::lan();
        assert_eq!(p.rate_kbit, 1_000_000);
        assert_eq!(p.rtt_ms(), 2); // 2 × 1 ms one-way
        assert!(!p.is_unshaped());
    }

    #[test]
    fn wan_profile_has_loss_and_jitter() {
        let p = NetemProfile::wan();
        assert_eq!(p.rate_kbit, 100_000);
        assert_eq!(p.rtt_ms(), 100);
        assert!(p.loss_pct > 0.0);
        assert!(p.jitter_ms > 0);
    }

    #[test]
    fn loopback_is_unshaped_and_needs_no_tc() {
        let p = NetemProfile::loopback();
        assert!(p.is_unshaped());
        assert_eq!(p.netem_args(), "");
        let (apply, _clear) = p.apply_commands("lo");
        assert!(apply.is_empty(), "unshaped baseline emits no tc apply");
    }

    #[test]
    fn netem_args_render_the_literature_profiles() {
        assert_eq!(
            NetemProfile::lan().netem_args(),
            "delay 1ms rate 1000000kbit"
        );
        assert_eq!(
            NetemProfile::wan().netem_args(),
            "delay 50ms 5ms rate 100000kbit loss 0.1%"
        );
    }

    #[test]
    fn apply_commands_target_the_named_iface() {
        let (apply, clear) = NetemProfile::wan().apply_commands("lo");
        assert!(apply.contains("tc qdisc add dev lo root netem"));
        assert!(apply.contains("delay 50ms 5ms"));
        assert!(clear.contains("tc qdisc del dev lo root"));
    }

    #[test]
    fn standard_set_has_loopback_lan_wan() {
        let set = NetemProfile::standard_set();
        assert_eq!(set.len(), 3);
        assert_eq!(set[0].id, "loopback");
        assert_eq!(set[1].id, "lan");
        assert_eq!(set[2].id, "wan");
    }

    #[test]
    fn trim_pct_drops_trailing_zeros() {
        assert_eq!(trim_pct(0.1), "0.1");
        assert_eq!(trim_pct(1.0), "1");
        assert_eq!(trim_pct(0.25), "0.25");
    }
}
