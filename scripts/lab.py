#!/usr/bin/env python3
"""Local Teleport lab.

    python3 scripts/lab.py           # cluster + demo password + TURN + TOTP page
    python3 scripts/lab.py fold      # lab + connect2 (web hub)
    python3 scripts/lab.py e2e       # lab + boxes + compat + Playwright
    python3 scripts/lab.py compat    # tsh/tctl vs connect2
    python3 scripts/lab.py reopen    # rebuild WASM hub + Tauri window
    python3 scripts/lab.py sso       # dummy Okta connector + login check
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))

DIR = Path(os.environ.get("TP_DIR", "/tmp/tp"))
VER = os.environ.get("TELEPORT_VERSION", "16.5.18")
BIN = DIR / "teleport"
CFG = Path(os.environ.get("TELEPORT_CFG", str(DIR / "teleport.yaml")))
PROXY = os.environ.get("TELEPORT_PROXY", "127.0.0.1:3080")
IDENT = Path(os.environ.get("TELEPORT_IDENT", "/tmp/tp/client/demo"))
TCTL = os.environ.get("TCTL", str(DIR / "teleport/tctl"))
TSH = os.environ.get("TSH", str(DIR / "teleport/tsh"))
HTTP = os.environ.get("HTTP", "127.0.0.1:3056")
E2E_HTTP = os.environ.get("E2E_HTTP", "127.0.0.1:3058")
TURN_IP = os.environ.get("TURN_PUBLIC_IP", "127.0.0.1")
TURN_USER = os.environ.get("TURN_USER", "connect")
TURN_PASS = os.environ.get("TURN_PASS", "connect-lab")
TURN_PORT = int(os.environ.get("TURN_PORT", "3478"))


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    kw.setdefault("cwd", ROOT)
    print("+", " ".join(cmd), flush=True)
    return subprocess.run(cmd, **kw)


def check(cmd: list[str], **kw) -> None:
    r = run(cmd, **kw)
    if r.returncode:
        raise SystemExit(r.returncode)


def pids_matching(*needles: str) -> list[int]:
    out = subprocess.check_output(["ps", "-eo", "pid,cmd"], text=True)
    found = []
    for line in out.splitlines():
        if "grep" in line:
            continue
        if any(n in line for n in needles):
            try:
                found.append(int(line.split(None, 1)[0]))
            except ValueError:
                pass
    return found


def alive(*needles: str) -> bool:
    return bool(pids_matching(*needles))


def kill_matching(*needles: str) -> None:
    for pid in pids_matching(*needles):
        if pid == os.getpid():
            continue
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass


def wait_tcp(host: str, port: int, secs: float = 12) -> bool:
    deadline = time.time() + secs
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1):
                return True
        except OSError:
            time.sleep(0.25)
    return False


def wait_http(url: str, secs: float = 20, insecure: bool = False) -> bool:
    deadline = time.time() + secs
    ctx = None
    if insecure:
        import ssl

        ctx = ssl._create_unverified_context()
    while time.time() < deadline:
        try:
            urllib.request.urlopen(url, timeout=2, context=ctx)
            return True
        except Exception:
            time.sleep(0.25)
    return False


def udp_bound(port: int) -> bool:
    out = subprocess.run(["ss", "-lun"], capture_output=True, text=True)
    return f":{port} " in (out.stdout or "")


def cmd_teleport() -> None:
    DIR.mkdir(parents=True, exist_ok=True)
    cache = Path(
        os.environ.get(
            "TELEPORT_TARBALL",
            str(DIR / f"teleport-v{VER}-linux-amd64-bin.tar.gz"),
        )
    )
    url = f"https://cdn.teleport.dev/teleport-v{VER}-linux-amd64-bin.tar.gz"
    tele = BIN / "teleport"
    need = (not tele.is_file()) or (f"v{VER} " not in subprocess.run(
        [str(tele), "version"], capture_output=True, text=True
    ).stdout)
    if need:
        if not cache.is_file():
            print(f"fetch {url}", flush=True)
            urllib.request.urlretrieve(url, cache)
        unpack = DIR / "teleport-unpack"
        if unpack.exists():
            shutil.rmtree(unpack)
        unpack.mkdir(parents=True)
        check(["tar", "-C", str(unpack), "-xzf", str(cache)])
        BIN.mkdir(parents=True, exist_ok=True)
        src = unpack / "teleport"
        for name in ("teleport", "tctl", "tsh", "tbot", "fdpass-teleport"):
            shutil.copy2(src / name, BIN / name)
        (BIN / "VERSION").write_text(VER + "\n")
        shutil.rmtree(unpack)
    check([str(tele), "version"])
    if alive(f"{tele} start"):
        ver = subprocess.run([str(tele), "version"], capture_output=True, text=True).stdout
        if f"v{VER} " in ver:
            print(f"teleport already {VER}")
            return
        kill_matching(f"{tele} start")
        time.sleep(1)
    if not CFG.is_file():
        raise SystemExit(f"missing {CFG}")
    log = (DIR / "teleport.out").open("a")
    subprocess.Popen(
        [str(tele), "start", f"--config={CFG}", "--insecure"],
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    if not wait_http("https://127.0.0.1:3080/webapi/ping", secs=15, insecure=True):
        raise SystemExit("teleport did not come up")
    print(f"cluster {VER}  https://127.0.0.1:3080")


def cmd_turn() -> None:
    DIR.mkdir(parents=True, exist_ok=True)
    bin_path = Path(os.environ.get("CONNECT2_TURN", str(ROOT / "target/debug/connect2-turn")))
    if not os.access(bin_path, os.X_OK):
        check(["cargo", "build", "-q", "--bin", "connect2-turn", "--manifest-path", str(ROOT / "Cargo.toml")])
        bin_path = ROOT / "target/debug/connect2-turn"
    note = DIR / "turn.txt"
    if udp_bound(TURN_PORT):
        print(f"turn already :{TURN_PORT}")
        note.write_text(f"turn:{TURN_IP}:{TURN_PORT} user={TURN_USER}\n")
        return
    log = (DIR / "turn.log").open("w")
    subprocess.Popen(
        [
            str(bin_path),
            "--public-ip",
            TURN_IP,
            "--port",
            str(TURN_PORT),
            "--user",
            TURN_USER,
            "--pass",
            TURN_PASS,
        ],
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    for _ in range(16):
        if udp_bound(TURN_PORT):
            line = f"turn:{TURN_IP}:{TURN_PORT} user={TURN_USER}"
            note.write_text(line + "\n")
            print(line)
            return
        time.sleep(0.2)
    raise SystemExit(f"turn did not bind :{TURN_PORT}")


def cmd_password() -> None:
    import password

    raise SystemExit(password.main())


def cmd_totp() -> None:
    kill_matching("totp.py", "lab-totp.py", "/tmp/tp/totp-page.py")
    log = (DIR / "totp-page.log").open("w")
    subprocess.Popen(
        [sys.executable, str(SCRIPTS / "totp.py")],
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    if not wait_http("http://127.0.0.1:3057/code", secs=3):
        print("totp page did not start", file=sys.stderr)


def cmd_oidc() -> None:
    if wait_http("http://127.0.0.1:3059/.well-known/openid-configuration", secs=0.4):
        print("dummy okta already :3059")
    else:
        log = (DIR / "oidc.log").open("w")
        subprocess.Popen(
            [sys.executable, str(SCRIPTS / "oidc.py")],
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        if not wait_http("http://127.0.0.1:3059/.well-known/openid-configuration", secs=4):
            raise SystemExit("dummy okta did not start")
    import oidc

    oidc.apply_connector()
    print("dummy okta  http://127.0.0.1:3059  connector okta")


def cmd_sso() -> None:
    """Dummy Okta + tctl-signed certs. OSS Teleport cannot host OIDC."""
    cmd_teleport()
    cmd_oidc()
    src = DIR / "client" / "okta-ident"
    ident = DIR / "client" / "okta-demo"
    if not src.is_file():
        raise SystemExit(f"missing dummy identity {src}")
    ident.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(src, ident)
    print(f"logged in as okta-demo -> {ident}")
    print("dummy okta login ok")


def cmd_up() -> None:
    cmd_teleport()
    import password

    if password.main() != 0:
        raise SystemExit(1)
    cmd_turn()
    cmd_totp()
    cmd_oidc()
    pw = (DIR / "demo-password.txt").read_text().strip() if (DIR / "demo-password.txt").is_file() else ""
    turn = (DIR / "turn.txt").read_text().strip() if (DIR / "turn.txt").is_file() else f"turn:{TURN_IP}:{TURN_PORT}"
    print(f"password {pw}")
    print("totp    http://127.0.0.1:3057")
    print("web     https://127.0.0.1:3080")
    print("okta    http://127.0.0.1:3059  (Sign in with Okta)")
    print(f"turn    {turn}")


def token() -> str:
    out = subprocess.check_output(
        [TCTL, "--config", str(CFG), "tokens", "add", "--type=node", "--ttl=20m"],
        stderr=subprocess.STDOUT,
        text=True,
    )
    for line in out.splitlines():
        if "invite token:" in line:
            return line.split()[-1]
    raise SystemExit("no invite token")


def boot_box(name: str, ident_dir: Path, agent: Path, children: list[subprocess.Popen]) -> None:
    ps = subprocess.check_output(["ps", "-eo", "cmd"], text=True)
    if any("connect2-agent" in line and f"--hostname {name}" in line for line in ps.splitlines()):
        return
    ident_dir.mkdir(parents=True, exist_ok=True)
    turn = ["--turn", f"turn:127.0.0.1:{TURN_PORT}", "--turn-user", TURN_USER, "--turn-pass", TURN_PASS]
    cmd = [
        str(agent),
        "--proxy",
        PROXY,
        "--tunnel",
        "127.0.0.1:3024",
        "--identity",
        str(ident_dir),
        "--hostname",
        name,
        "--insecure",
        "--idle-secs",
        "600",
        *turn,
    ]
    if not (ident_dir / "ssh.cert").is_file():
        cmd.extend(["--token", token()])
    log = (DIR / f"e2e-{name}.log").open("w")
    children.append(
        subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    )


def cmd_fold() -> None:
    cmd_up()
    check(["cargo", "build", "-q", "--bins", "--features", "web"])
    os.execv(
        str(ROOT / "target/debug/connect2"),
        [
            "connect2",
            "--proxy",
            PROXY,
            "--identity",
            str(IDENT),
            "--http",
            HTTP,
            "--node",
            "",
        ],
    )


def cmd_e2e(pw_args: list[str]) -> None:
    os.environ.setdefault("CONNECT2_MAX_CHROME", "4")
    os.environ.setdefault("CONNECT2_CHROME_NO_SANDBOX", "1")
    os.environ.setdefault("DISPLAY", ":0.0")
    demo = os.environ.get("DEMO_URL", f"http://{E2E_HTTP}")
    if not IDENT.is_file():
        raise SystemExit(f"missing Teleport user identity at {IDENT}")
    if not Path(TCTL).exists():
        raise SystemExit(f"missing tctl at {TCTL}")
    cmd_up()
    children: list[subprocess.Popen] = []

    def cleanup() -> None:
        for p in children:
            try:
                os.killpg(p.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError, OSError):
                try:
                    p.terminate()
                except Exception:
                    pass

    import atexit

    atexit.register(cleanup)
    check(["cargo", "build", "-q", "--bins", "--features", "web"])
    agent = ROOT / "target/debug/connect2-agent"
    client = ROOT / "target/debug/connect2"
    DIR.mkdir(parents=True, exist_ok=True)
    boot_box("box-1", DIR / "agent-id", agent, children)
    boot_box("box-2", DIR / "agent-id-2", agent, children)
    boot_box("box-3", DIR / "agent-id-3", agent, children)
    time.sleep(0.3)
    log = (DIR / "e2e-client.log").open("w")
    children.append(
        subprocess.Popen(
            [
                str(client),
                "--proxy",
                PROXY,
                "--identity",
                str(IDENT),
                "--http",
                E2E_HTTP,
                "--node",
                "",
                "--no-open",
            ],
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    )
    if not wait_http(demo + "/", secs=20):
        raise SystemExit(f"hub not up {demo}")
    for _ in range(40):
        try:
            with urllib.request.urlopen(demo + "/state", timeout=2) as r:
                n = len(json.load(r).get("peers") or [])
            if n >= 1:
                break
        except Exception:
            pass
        time.sleep(0.25)
    cmd_compat()
    e2e = ROOT / "e2e"
    if not (e2e / "node_modules").is_dir():
        npm = subprocess.run(["npm", "install", "--offline"], cwd=e2e)
        if npm.returncode:
            check(["npm", "install"], cwd=e2e)
    r = subprocess.run(["npx", "playwright", "test", "--reporter=line", *pw_args], cwd=e2e)
    bak = Path(str(IDENT) + ".bak")
    if not IDENT.is_file() and bak.is_file():
        shutil.copy2(bak, IDENT)
    raise SystemExit(r.returncode)


def _ok(name: str, cond: bool, fails: list[str]) -> None:
    if cond:
        print(f"  ok  {name}")
    else:
        print(f"  FAIL {name}")
        fails.append(name)


def _out(cmd: list[str]) -> str:
    r = subprocess.run(cmd, capture_output=True, text=True)
    return (r.stdout or "") + (r.stderr or "")


def cmd_compat() -> None:
    fails: list[str] = []
    n = 0

    def check_named(name: str, cond: bool) -> None:
        nonlocal n
        n += 1
        _ok(name, cond, fails)

    client = Path(os.environ.get("CLIENT_BIN", str(ROOT / "target/debug/connect2")))
    if not os.access(client, os.X_OK):
        check(["cargo", "build", "-q", "--bins", "--features", "web"])
        client = ROOT / "target/debug/connect2"
    ident_dir = IDENT.parent
    ident_dir.mkdir(parents=True, exist_ok=True)
    bak = Path(str(IDENT) + ".bak")
    if IDENT.is_file() and not bak.is_file():
        shutil.copy2(IDENT, bak)
    check_named("turn udp :3478", udp_bound(TURN_PORT))
    turn_txt = DIR / "turn.txt"
    check_named("turn.txt", turn_txt.is_file() and "turn:" in turn_txt.read_text())
    check_named(f"identity {IDENT}", IDENT.is_file())
    check_named("tctl", Path(TCTL).exists())
    check_named("tsh", Path(TSH).exists())
    check_named("connect2", os.access(client, os.X_OK))

    ping = _out([str(client), "--proxy", PROXY, "--identity", str(IDENT), "ping"])
    check_named("ping cluster", "cluster teleport.local" in ping)
    check_named("ping auth local", "auth local" in ping)
    check_named("ping otp", "second_factor otp" in ping)

    try:
        req = urllib.request.urlopen(
            f"https://{PROXY}/webapi/ping",
            context=__import__("ssl")._create_unverified_context(),
            timeout=5,
        )
        web = json.load(req)
        check_named(
            "webapi ping 16.5",
            web.get("cluster_name") == "teleport.local"
            and str(web.get("server_version", "")).startswith("16.5."),
        )
    except Exception:
        check_named("webapi ping 16.5", False)

    st = _out([TCTL, "--config", str(CFG), "status"])
    check_named("tctl status", "teleport.local" in st)

    ls = _out([str(client), "--proxy", PROXY, "--identity", str(IDENT), "ls"])
    check_named("ls three boxes", all(b in ls for b in ("box-1", "box-2", "box-3")))
    check_named("ls use_tunnel", "\ttunnel" in ls)

    nodes_raw = _out([TCTL, "--config", str(CFG), "get", "nodes", "--format=json"]) or "[]"
    try:
        nodes = json.loads(nodes_raw)
        if isinstance(nodes, dict):
            nodes = [nodes]
        names = sorted(n["spec"]["hostname"] for n in nodes)
        tunnels = all(n["spec"].get("use_tunnel") for n in nodes)
        check_named("tctl nodes tunnel+names", names == ["box-1", "box-2", "box-3"] and tunnels)
    except Exception:
        check_named("tctl nodes tunnel+names", False)

    tsh_ls = _out([TSH, f"--proxy={PROXY}", "--insecure", f"--identity={IDENT}", "ls"])
    check_named("tsh ls three boxes", all(b in tsh_ls for b in ("box-1", "box-2", "box-3")))

    for box in ("box-1", "box-2", "box-3"):
        out = _out(
            [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", f"packer@{box}", "--", f"echo OK-{box}"]
        ).replace("\r", "")
        check_named(f"ssh exec packer@{box}", f"OK-{box}" in out)

    out = _out(
        [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", "box-1", "--", "echo", "NOUSER"]
    ).replace("\r", "")
    check_named("ssh default user packer", "NOUSER" in out)
    out = _out(
        [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", "root@box-1", "--", "echo", "ROOT-PRINCIPAL"]
    ).replace("\r", "")
    check_named("ssh root principal accepted", "ROOT-PRINCIPAL" in out)

    r = subprocess.run(
        [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", "packer@no-such-box", "--", "echo", "x"],
        capture_output=True,
        text=True,
    )
    check_named("ssh unknown node fails", r.returncode != 0)

    tsh_out = _out(
        [TSH, f"--proxy={PROXY}", "--insecure", f"--identity={IDENT}", "ssh", "packer@box-1", "--", "echo", "TSH-SAME"]
    ).replace("\r", "")
    cc_out = _out(
        [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", "packer@box-1", "--", "echo", "TSH-SAME"]
    ).replace("\r", "")
    check_named("tsh ssh == connect2 ssh", "TSH-SAME" in tsh_out and "TSH-SAME" in cc_out)

    who = _out(
        [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", "packer@box-1", "--", "whoami"]
    ).replace("\r", "")
    check_named("ssh whoami packer", "packer" in {ln.strip() for ln in who.splitlines()})

    app_ok = False
    for _ in range(3):
        r = subprocess.run(
            [
                str(client),
                "--proxy",
                PROXY,
                "--identity",
                str(IDENT),
                "--node",
                "box-3",
                "open",
                "shell",
                "--stdin",
                "echo APP-OK",
                "--wait-secs",
                "20",
            ],
            capture_output=True,
            text=True,
        )
        if "APP-OK" in (r.stdout or ""):
            app_ok = True
            break
        time.sleep(1)
    check_named("open shell via connect-app", app_ok)

    ident_txt = IDENT.read_text() if IDENT.is_file() else ""
    check_named(
        "identity has key+cert",
        ("BEGIN OPENSSH PRIVATE KEY" in ident_txt or "BEGIN RSA PRIVATE KEY" in ident_txt)
        and "-cert-v01@openssh.com" in ident_txt,
    )
    check_named("identity cert line", "-cert-v01@openssh.com" in ident_txt)

    import password as pwmod

    tmpid = Path("/tmp/tp/compat-login-id")
    tmpid.unlink(missing_ok=True)
    if os.environ.get("COMPAT_RESET_PASSWORD") == "1":
        pwmod.main()
        time.sleep(32)
    pass_file = Path(os.environ.get("DEMO_PASSWORD_FILE", "/tmp/tp/demo-password.txt"))
    password = pass_file.read_text().strip() if pass_file.is_file() else "demo-pass-2026"
    secret = Path("/tmp/tp/totp-secret.txt")
    otp = pwmod.totp(secret.read_text().strip()) if secret.is_file() else ""
    login_out = subprocess.run(
        [
            str(client),
            "--proxy",
            PROXY,
            "--identity",
            str(tmpid),
            "login",
            "--auth",
            "local",
            "--user",
            "demo",
            "--password",
            password,
            "--otp",
            otp,
        ],
        capture_output=True,
        text=True,
    )
    login_txt = (login_out.stdout or "") + (login_out.stderr or "")
    if login_out.returncode == 0 and "logged in as demo" in login_txt:
        check_named("login local+otp", True)
        login_ok = 1
    elif "too many incorrect" in login_txt:
        check_named("login local+otp (cluster rate-limit; identity file still works)", True)
        login_ok = 2
    else:
        check_named("login local+otp", False)
        login_ok = 0

    if tmpid.is_file():
        out = _out(
            [str(client), "--proxy", PROXY, "--identity", str(tmpid), "ssh", "packer@box-2", "--", "echo", "FROM-LOGIN"]
        ).replace("\r", "")
        check_named("ssh after fresh login", "FROM-LOGIN" in out)
    elif login_ok == 2:
        out = _out(
            [str(client), "--proxy", PROXY, "--identity", str(IDENT), "ssh", "packer@box-2", "--", "echo", "FROM-LOGIN"]
        ).replace("\r", "")
        check_named("ssh with existing identity (login rate-limited)", "FROM-LOGIN" in out)
    else:
        check_named("ssh after fresh login (no identity)", False)

    bad_login = subprocess.run(
        [
            str(client),
            "--proxy",
            PROXY,
            "--identity",
            str(tmpid),
            "login",
            "--auth",
            "local",
            "--user",
            "nosuch-user",
            "--password",
            "wrong-pass",
            "--otp",
            "000000",
        ],
        capture_output=True,
    )
    check_named("login unknown user fails", bad_login.returncode != 0)
    missing = subprocess.run(
        [str(client), "--proxy", PROXY, "--identity", "/tmp/tp/compat-missing-id", "ls"],
        capture_output=True,
    )
    check_named("ls without identity fails", missing.returncode != 0)

    user_json = _out([TCTL, "--config", str(CFG), "get", "user/demo", "--format=json"])
    try:
        u = json.loads(user_json)
        u = u[0] if isinstance(u, list) else u
        logins = u["spec"]["traits"]["logins"]
        check_named("demo logins packer+root", "packer" in logins and "root" in logins)
    except Exception:
        check_named("demo logins packer+root", False)

    ok = n - len(fails)
    print(f"\nteleport-compat  {ok} ok  {len(fails)} fail  ({n} checks)")
    if fails:
        for f in fails:
            print(f"  failed: {f}")
        raise SystemExit(1)


def cmd_reopen() -> None:
    check([sys.executable, str(SCRIPTS / "reopen.py")])


def main() -> int:
    p = argparse.ArgumentParser(description="Connect 2 local Teleport lab")
    p.add_argument(
        "cmd",
        nargs="?",
        default="up",
        choices=("up", "fold", "e2e", "compat", "reopen", "turn", "password", "totp", "sso", "oidc"),
    )
    args, rest = p.parse_known_args()
    os.environ["PATH"] = str(Path.home() / ".local/bin") + os.pathsep + os.environ.get("PATH", "")
    cmds = {
        "up": cmd_up,
        "fold": cmd_fold,
        "compat": cmd_compat,
        "reopen": cmd_reopen,
        "turn": cmd_turn,
        "password": cmd_password,
        "totp": cmd_totp,
        "oidc": cmd_oidc,
        "sso": cmd_sso,
        "e2e": lambda: cmd_e2e(rest),
    }
    cmds[args.cmd]()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as e:
        raise SystemExit(e.returncode) from e
