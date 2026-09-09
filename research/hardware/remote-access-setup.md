# Remote-access runbook — drive the Dell XPS (Ubuntu + NVIDIA) from the M1 Mac

**Audience:** you (the human), one-time setup. After this, the coding agent running
as a process on your **M1 Mac** can SSH into your **Dell XPS 15 9500 (Ubuntu, GTX
1650 Ti)** to build, run, and benchmark sparq on real NVIDIA/CUDA hardware — the
one thing the Mac cannot do.

**Mental model:** the agent only runs local shell commands on the Mac. To use the
XPS, it runs `ssh xps '<command>'`. So the entire goal of this runbook is: make
`ssh xps` work non-interactively (key-based, no password prompt) from the Mac, even
if the two machines are on different networks, and install the toolchain on the XPS.

Terminology used below:
- **`MAC$`** = run this in a terminal on the M1 Mac (you).
- **`XPS$`** = run this in a terminal on the Dell XPS (you, physically or via SSH).

---

## Part 0 — Pick your connectivity path

| Situation | Use | Why |
|---|---|---|
| Both machines on the **same home/office LAN/Wi-Fi** | **Plain SSH over LAN** (§1) | Simplest. Just need the XPS's LAN IP. |
| Different networks / XPS behind NAT / you travel | **Tailscale** (§2, RECOMMENDED) | Zero-config mesh VPN, stable hostname, survives IP changes, encrypted. Best overall. |
| Can't install Tailscale / want a quick one-off | **Cloudflare Tunnel** (§3) | `cloudflared` is already installed on your Mac. Good fallback. |

**Recommendation: set up Tailscale (§2).** It gives a permanent name `xps` that
works from anywhere, so the agent's `ssh xps '...'` commands keep working whether
you're home or travelling. Do §1 first only if both boxes are on the same LAN right
now and you want to test in 5 minutes.

> The SSH key step (§1.1) is required for **all** paths — do it regardless.

---

## Part 1 — SSH key + base SSH config (required for every path)

### 1.1 Create an SSH key on the Mac (if you don't already have one)

```bash
MAC$ ls ~/.ssh/id_ed25519.pub 2>/dev/null && echo "key exists, skip keygen" || \
     ssh-keygen -t ed25519 -C "mac-to-xps" -f ~/.ssh/id_ed25519 -N ""
```

(`-N ""` = no passphrase, so the agent can connect non-interactively. If you
prefer a passphrase, add the key to the agent once per login with
`ssh-add --apple-use-keychain ~/.ssh/id_ed25519`.)

### 1.2 Prepare the XPS: install + enable SSH server

Do this once, physically at the XPS (or however you currently reach it):

```bash
XPS$ sudo apt update
XPS$ sudo apt install -y openssh-server
XPS$ sudo systemctl enable --now ssh
XPS$ whoami        # note your username, e.g. "jesse"
XPS$ hostname -I   # note the LAN IP if you'll use plain SSH, e.g. 192.168.1.42
```

### 1.3 Copy the Mac's public key to the XPS

If the XPS is reachable on the LAN right now:

```bash
MAC$ ssh-copy-id -i ~/.ssh/id_ed25519.pub <xps-user>@<xps-lan-ip>
# e.g. ssh-copy-id -i ~/.ssh/id_ed25519.pub jesse@192.168.1.42
```

If not reachable yet, do it after Tailscale/Cloudflare is up (substitute the
Tailscale hostname for the IP). Or paste manually:

```bash
MAC$ cat ~/.ssh/id_ed25519.pub          # copy this line
XPS$ mkdir -p ~/.ssh && chmod 700 ~/.ssh
XPS$ echo "<paste the line>" >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys
```

### 1.4 Add an `xps` alias to the Mac's SSH config

This is what lets the agent type `ssh xps` instead of a full host string. Create or
append to `~/.ssh/config` on the Mac:

```bash
MAC$ cat >> ~/.ssh/config <<'EOF'

Host xps
    HostName <FILL-IN>          # LAN IP (§1), or Tailscale name (§2), or 127.0.0.1 + Port (§3)
    User <xps-user>             # e.g. jesse
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 30      # keep long benchmark sessions alive
    ServerAliveCountMax 6
    # Port 22                   # uncomment + set if using Cloudflare local-forward (§3)
EOF
MAC$ chmod 600 ~/.ssh/config
```

Fill in `HostName` per whichever path you chose (you'll update it in §2/§3).

### 1.5 Verify

```bash
MAC$ ssh xps 'echo connected as $(whoami) on $(hostname); uname -a'
```

If that prints the XPS's hostname **without** asking for a password, base SSH is
done. The agent can now drive the XPS.

---

## Part 2 — Tailscale (RECOMMENDED cross-network path)

A free mesh VPN. Gives the XPS a stable name reachable from the Mac anywhere.

### 2.1 Install + log in on both machines

```bash
# On the XPS:
XPS$ curl -fsSL https://tailscale.com/install.sh | sh
XPS$ sudo tailscale up
#   -> prints a URL; open it, log in (Google/GitHub/email). Authorises this node.
XPS$ tailscale ip -4        # note the 100.x.y.z address
XPS$ tailscale status       # confirm it's up

# On the Mac:
MAC$ brew install --cask tailscale   # or download the app from tailscale.com
MAC$ # launch Tailscale.app and log in with the SAME account
MAC$ tailscale status                # should list "xps" once both are up
```

Both nodes must be logged into the **same Tailscale account** (tailnet).

### 2.2 Point the SSH alias at the Tailscale name

Tailscale gives each node a MagicDNS name (usually the machine's hostname). Use
that, or the `100.x` IP:

```bash
MAC$ ssh xps 'true' 2>/dev/null && echo "already works" || true
# Edit ~/.ssh/config: set  HostName  to the XPS's tailscale name or 100.x.y.z
#   HostName xps              # if MagicDNS is on and the node is named "xps"
#   HostName 100.101.102.103  # otherwise the tailscale IP from `tailscale ip -4`
MAC$ ssh xps 'echo tailscale-ok; hostname'
```

(Optional, even slicker: `sudo tailscale up --ssh` on the XPS enables Tailscale's
own SSH — but the key-based OpenSSH set up in Part 1 is fine and more standard.)

---

## Part 3 — Cloudflare Tunnel (fallback; `cloudflared` already on your Mac)

Use this if you can't/won't use Tailscale. It exposes the XPS's SSH port to the Mac
through Cloudflare's edge. `cloudflared` is already installed on the Mac
(`/opt/homebrew/bin/cloudflared`).

### 3.1 On the XPS — run a tunnel for SSH

Quick (ephemeral) tunnel, no Cloudflare account needed for a one-off:

```bash
XPS$ sudo apt install -y cloudflared   # or download from Cloudflare
XPS$ cloudflared tunnel --url ssh://localhost:22
#   -> prints a https://<random>.trycloudflare.com hostname. Copy it.
```

(For a stable, named tunnel that survives reboots, create a free Cloudflare
account and `cloudflared tunnel login` + `tunnel create xps` + a config file with
`ingress: ssh://localhost:22` + `cloudflared service install`. The ephemeral form
above is enough to start.)

### 3.2 On the Mac — connect SSH through the tunnel

Add a `ProxyCommand` so OpenSSH dials the Cloudflare hostname:

```bash
# In ~/.ssh/config, replace the xps block's HostName with the tunnel host and add:
#   Host xps
#       HostName <random>.trycloudflare.com
#       User <xps-user>
#       IdentityFile ~/.ssh/id_ed25519
#       ProxyCommand /opt/homebrew/bin/cloudflared access ssh --hostname %h
MAC$ ssh xps 'echo cloudflare-tunnel-ok'
```

> Note: ephemeral `trycloudflare.com` hostnames change every time you restart the
> tunnel — you'll re-edit `HostName`. A named tunnel (account required) gives a
> fixed hostname. For an always-on agent workflow, prefer Tailscale (§2).

### Alternative: reverse SSH (`ssh -R`) with no third party

If you have *any* always-on box with a public IP (a cheap VPS), the XPS can dial
out to it and expose its SSH back:

```bash
XPS$ ssh -N -R 2222:localhost:22 user@your-vps      # XPS -> VPS, reverse-forward
MAC$ # ~/.ssh/config xps block: HostName your-vps , Port 2222
```

This avoids Cloudflare/Tailscale but needs you to own a public-IP host. Tailscale
is simpler; listed for completeness.

---

## Part 4 — One-time toolchain install on the XPS (Rust + CUDA + driver checks)

Run these on the XPS (directly, or via `ssh xps '...'` once Part 1 works).

### 4.1 Verify the NVIDIA driver + GPU are alive

```bash
XPS$ nvidia-smi
#   Expect a table naming "GeForce GTX 1650 Ti", a driver version, and ~4096 MiB
#   total memory. If "command not found" or "NVIDIA-SMI has failed", install the
#   driver:
XPS$ ubuntu-drivers devices            # see recommended driver
XPS$ sudo ubuntu-drivers autoinstall   # installs the recommended NVIDIA driver
XPS$ sudo reboot                        # driver needs a reboot
# after reboot:
XPS$ nvidia-smi                         # must now show the GPU
```

> On Optimus laptops like the XPS, the discrete GPU may be in "on-demand" mode.
> `nvidia-smi` should still see it. If a CUDA program can't find the GPU, prefix it
> with `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia` or check
> `prime-select`.

### 4.2 Install Rust

```bash
XPS$ curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
XPS$ source "$HOME/.cargo/env"
XPS$ rustc --version && cargo --version
XPS$ sudo apt install -y build-essential pkg-config git    # linker, headers, git
```

### 4.3 Install the CUDA toolkit (only needed for the `cudarc`/CUDA backend)

The **wgpu path needs no CUDA toolkit** — it uses Vulkan, which the NVIDIA driver
already provides. Install CUDA only when you start the `cudarc` backend.

```bash
XPS$ sudo apt install -y nvidia-cuda-toolkit    # simplest; gives nvcc + libs
XPS$ nvcc --version                              # confirm toolkit version
# (For the latest CUDA 12.x/13.x, install NVIDIA's official .deb repo instead of
#  Ubuntu's packaged toolkit — but apt's version is fine to start.)
```

For Vulkan validation (what wgpu uses):

```bash
XPS$ sudo apt install -y vulkan-tools
XPS$ vulkaninfo --summary | grep -i "deviceName\|driverName"   # should list the NVIDIA GPU
```

### 4.4 Get the sparq source onto the XPS

```bash
XPS$ git clone https://github.com/sparq-org/sparq.git ~/sparq    # your fork/remote
XPS$ cd ~/sparq && cargo build --release                     # first build (CPU)
```

(Or sync from the Mac's working copy instead of GitHub — see §5.2.)

---

## Part 5 — How the agent then drives the XPS

Once `ssh xps` works, these are the exact patterns the agent (or you) will use.

### 5.1 Run remote commands

```bash
# Build the engine on the XPS:
MAC$ ssh xps 'cd ~/sparq && source ~/.cargo/env && cargo build --release'

# Run the test suite:
MAC$ ssh xps 'cd ~/sparq && cargo test --release'

# Run a query / benchmark binary:
MAC$ ssh xps 'cd ~/sparq && ./target/release/sparq-cli ...'

# Confirm the GPU before a GPU run:
MAC$ ssh xps 'nvidia-smi --query-gpu=name,memory.total,memory.used --format=csv'
```

Tip: wrap long commands with `source ~/.cargo/env` (non-login SSH shells may not
have cargo on PATH), or add `. "$HOME/.cargo/env"` to `~/.bashrc` on the XPS so it's
always present.

### 5.2 Copy files both ways (rsync preferred, scp fine)

```bash
# Push the Mac's working tree to the XPS (fast, incremental; skips build + git):
MAC$ rsync -avz --delete \
        --exclude target/ --exclude .git/ --exclude '*.nt' --exclude '*.ttl' \
        ~/Documents/GitHub/rdfjs/sparq/  xps:~/sparq/

# Pull benchmark results / artifacts back to the Mac:
MAC$ rsync -avz xps:~/sparq/bench/results/  ~/Documents/GitHub/rdfjs/sparq/bench/results-xps/

# One-off single file:
MAC$ scp ~/some-dataset.nt xps:~/sparq/data/
MAC$ scp xps:~/sparq/out.json ./
```

`rsync` over `git push/pull` is the right loop for iterating: edit on the Mac, push
working tree, build+bench on the XPS, pull results — no commits needed.

### 5.3 Run a GPU benchmark (the wgpu spike or future GPU path)

```bash
# Copy the standalone wgpu spike to the XPS and run it on the NVIDIA GPU:
MAC$ rsync -avz --exclude target/ \
        ~/Documents/GitHub/rdfjs/sparq/research/hardware/wgpu-spike/ xps:~/wgpu-spike/
MAC$ ssh xps 'cd ~/wgpu-spike && source ~/.cargo/env && cargo run --release'
#   -> on the XPS this should print  backend: Vulkan  /  name: NVIDIA GeForce GTX 1650 Ti
#      i.e. the SAME kernel that ran on Metal here now runs on the NVIDIA GPU.
#      Compare its cpu/gpu crossover numbers to the M1 Metal numbers in gpu-and-cloud.md.

# Force the discrete GPU if Vulkan picks the integrated/Intel device:
MAC$ ssh xps 'cd ~/wgpu-spike && source ~/.cargo/env && \
              __NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia cargo run --release'
```

### 5.4 Long-running benchmarks that outlive the SSH session

Use `tmux` (or `nohup`) on the XPS so a dropped connection doesn't kill a 30-minute
index build:

```bash
MAC$ ssh xps 'sudo apt install -y tmux'                       # once
MAC$ ssh xps 'tmux new -d -s bench "cd ~/sparq && cargo run --release -p sparq-bench -- --scale 5000000 |& tee ~/bench.log"'
MAC$ ssh xps 'tmux capture-pane -pt bench | tail -40'         # peek at progress
MAC$ rsync -avz xps:~/bench.log ./                            # pull the log
```

---

## Part 6 — Security notes

- **Key-based auth only — disable passwords on the XPS** once your key works:
  ```bash
  XPS$ sudo sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
  XPS$ sudo sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
  XPS$ sudo systemctl restart ssh
  ```
- **Do not expose port 22 to the public internet.** With Tailscale (§2) the SSH
  port is only reachable inside your private tailnet — nothing is published. With
  Cloudflare Tunnel (§3) only the `cloudflared`-authenticated path reaches it, not
  a raw open port. Prefer these over port-forwarding your router to the XPS.
- **The `-N ""` passphrase-less key** is a convenience that trades a bit of safety
  for non-interactive agent use. The private key never leaves the Mac. If the Mac
  is shared or you want more safety, use a passphrase + `ssh-add
  --apple-use-keychain` once per login; the agent then inherits the loaded key.
- **Scope what the agent can do.** The agent runs as your XPS user. If you want a
  blast-radius limit, create a dedicated `sparq` user on the XPS, clone the repo
  there, and put *that* user in `~/.ssh/config` — it can build/bench but isn't your
  primary account. `sudo` (driver/CUDA installs in Part 4) would then need your
  main account.
- **Tailscale ACLs / Cloudflare Access** let you further restrict which devices or
  identities can reach the XPS; the defaults (same-tailnet-only / your Cloudflare
  account) are already private.
- **`tmux`/`nohup` benchmarks keep running after you disconnect** — remember to
  `tmux kill-session -t bench` or they'll hold the GPU/CPU.

---

## Quick checklist

```text
[ ] §1.1  ssh key exists on Mac
[ ] §1.2  openssh-server running on XPS
[ ] §1.3  Mac public key in XPS ~/.ssh/authorized_keys
[ ] §1.4  `xps` block in Mac ~/.ssh/config
[ ] §2/§3 connectivity (Tailscale up on both, OR cloudflared tunnel)
[ ] §1.5  `ssh xps 'echo ok'` works with NO password prompt
[ ] §4.1  `ssh xps 'nvidia-smi'` shows the GTX 1650 Ti
[ ] §4.2  `ssh xps 'cargo --version'` works
[ ] §4.4  ~/sparq cloned + `cargo build --release` succeeds on XPS
[ ] §5.3  wgpu spike prints "backend: Vulkan / NVIDIA GeForce GTX 1650 Ti"
[ ] §6    PasswordAuthentication no on XPS
```

---

## Recommended next step

Do **§1 + §2 (Tailscale) + §1.5 verify** first — that single `ssh xps 'echo ok'`
working non-interactively is the unlock; everything else (toolchain, GPU runs) the
agent can then drive remotely. As soon as it works, run
`ssh xps 'nvidia-smi'` and paste the output back so the GPU model, driver, and
exact VRAM are confirmed against the assumptions in `gpu-and-cloud.md` (which
assumes a 4 GB GTX 1650 Ti Mobile) — then the agent can `rsync` the wgpu spike over
and produce the **first real NVIDIA-vs-M1 portable-kernel comparison.**
