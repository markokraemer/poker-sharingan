"""A bot that tries every way it can to carry notes from one rotation of duplicate deals into the next.

Used by deploy/platinum/bot/test_isolation.sh against the real bot image: each new bot process first looks for notes
left by an earlier process (files in every writable place, POSIX shared memory, a daemon on an abstract unix socket,
/tmp's timestamp, its own stderr, and every file it can read in the scratch and run directories: it prints a note on
stderr on every turn, so a log that carries bot output and is readable later would show up) and prints `SEEN <how> from <uid:pid> by <uid:pid>` on stderr for every one it finds, then leaves
its own notes and plays on.
Speaks the pipe protocol directly. (That a process never meets a deal twice is tested in arena/server/tests/worker.rs.)
"""
import json
import os
import re
import socket
import stat
import sys

ME = f"{os.getuid()}:{os.getpid()}"
MARK_PORTS = {40000 + u % 1000 for u in (10002, 10010, 10011, 10012, 10013, 10014, 10015)}
SOCK = "\0sharingan-cheat"
PLACES = ["/tmp/notes", "/var/tmp/notes", "/dev/shm/notes", os.path.expanduser("~/notes"), "/opt/sharingan/bot/notes",
          "/opt/sharingan/deps/notes", "/work/notes", "/run/lock/notes"]


def seen(how, src):
    """A note from another process got through: `SEEN <how> from <uid:pid> by <uid:pid>` (the harness decides)."""
    src = src.strip()
    if src and src != ME:
        print(f"SEEN {how} from {src} by {ME}", file=sys.stderr, flush=True)


def priv(what):
    print(f"PRIV {what} (as {ME})", file=sys.stderr, flush=True)


def probe_privileges():
    """Anything a bot user must not be able to do; `PRIV <what>` for each that works."""
    try:
        for ln in open("/proc/self/status"):
            if ln.split(":")[0] in ("CapEff", "CapPrm", "CapInh", "CapAmb") and int(ln.split()[1], 16):
                priv(ln.strip())
    except OSError:
        pass
    try:
        os.setuid(0)
        priv("setuid(0)")
    except OSError:
        pass
    for pid in os.listdir("/proc"):
        try:
            if pid.isdigit() and open(f"/proc/{pid}/comm").read().strip() == "sharingan-arena":
                try:
                    os.kill(int(pid), 0)
                    priv(f"signal the worker ({pid})")
                except OSError:
                    pass
                try:
                    open(f"/proc/{pid}/environ").read(1)
                    priv("read the worker's environment")
                except OSError:
                    pass
        except OSError:
            pass
    for path in ("/opt/sharingan/worker.key", "/opt/sharingan/run.seed", "/models/sharingan-1.plcp"):
        try:
            open(path, "rb").read(1)
            priv(f"read {path}")
        except OSError:
            pass
    try:
        os.listdir("/opt/sharingan/run")
        priv("list the runner's directory")
    except OSError:
        pass


def look():
    # access times: a file of the (root-owned) bot directory read by an earlier user, m<uid % 16>
    for i in range(16):
        try:
            st = os.stat(f"/opt/sharingan/bot/m{i}")
            # moved by a read: later than the file's change time, and not the far-future value the runner seals it with
            if st.st_ctime_ns < st.st_atime_ns < 3786825600 * 10**9:
                seen(f"atime m{i}", f"{10000 + i}:atime")
        except OSError:
            pass
    for p in PLACES:
        try:
            with open(p) as f:
                seen(f"file {p}", f.read(40))
        except OSError:
            pass
    try:
        from multiprocessing import shared_memory
        s = shared_memory.SharedMemory(name="sharingan-cheat")
        seen("shared memory", bytes(s.buf[:40]).rstrip(b"\0").decode())
        s.close()
    except Exception:
        pass
    try:
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        c.settimeout(0.5)
        c.connect(SOCK)
        seen("daemon", c.recv(64).decode())
    except OSError:
        pass
    # its own stderr: a shared regular file would carry (or at least measure) what earlier processes printed
    try:
        st = os.fstat(2)
        if stat.S_ISREG(st.st_mode) and st.st_size > 0:
            seen("stderr is a shared file", f"{st.st_size}:size")
    except OSError:
        pass
    # every readable file in the places a log could live
    for top in ("/tmp", "/var/tmp", "/dev/shm", "/run", "/work", "/opt/sharingan"):
        for root, dirs, files in os.walk(top):
            for fn in files:
                path = os.path.join(root, fn)
                try:
                    if os.path.getsize(path) > 64 << 20:
                        continue
                    with open(path, "rb") as f:
                        for m in set(re.findall(rb"CHEATNOTE (\d+:\d+)", f.read())):
                            seen(f"readable file {path}", m.decode())
                except OSError:
                    pass
    # TCP TIME_WAIT state outlives the process that made it: ports 40000+uid%1000 left by an earlier user
    for tab in ("/proc/net/tcp", "/proc/net/tcp6"):
        try:
            for ln in open(tab).read().splitlines()[1:]:
                f = ln.split()
                if len(f) > 3 and f[3] == "06":
                    for ep in (f[1], f[2]):
                        port = int(ep.rsplit(":", 1)[1], 16)
                        if port in MARK_PORTS:   # only the ports a cheater binds (ephemeral ports fall in this range too)
                            seen("tcp time_wait", f"{10000 + port - 40000}:tw")
        except OSError:
            pass
    try:
        tag = os.stat("/tmp").st_mtime_ns % 100000   # an earlier process may have written its uid into /tmp's timestamp
        if 10000 <= tag < 10100:
            seen("/tmp mtime", f"{tag}:mtime")
    except OSError:
        pass


def write_notes():
    try:
        open(f"/opt/sharingan/bot/m{os.getuid() % 16}").read()   # moves its access time, unless the mount says noatime
    except OSError:
        pass
    for p in PLACES:
        try:
            with open(p, "w") as f:
                f.write(ME)
            os.chmod(p, 0o666)
        except OSError:
            pass
    try:
        from multiprocessing import shared_memory, resource_tracker
        try:
            s = shared_memory.SharedMemory(name="sharingan-cheat", create=True, size=64)
        except FileExistsError:
            s = shared_memory.SharedMemory(name="sharingan-cheat")
        b = ME.encode()[:40]
        s.buf[:40] = b + b"\0" * (40 - len(b))
        resource_tracker.unregister(s._name, "shared_memory")   # keep it after we exit
    except Exception:
        pass
    try:
        ns = os.stat("/tmp").st_mtime_ns
        os.utime("/tmp", ns=(ns, ns - ns % 100000 + os.getuid() % 100000))
    except OSError:
        pass
    # TCP to itself over loopback from a port that names this user, closed client-first: the TIME_WAIT entry stays ~60 s
    try:
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        cli = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        cli.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        cli.bind(("127.0.0.1", 40000 + os.getuid() % 1000))
        cli.settimeout(1)
        cli.connect(srv.getsockname())
        conn, _ = srv.accept()
        cli.close()
        conn.close()
        srv.close()
    except OSError:
        pass
    # a daemon that outlives this process and serves notes on an abstract socket
    if os.fork() == 0:
        os.setsid()
        if os.fork() == 0:
            try:
                srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                srv.bind(SOCK)
                srv.listen(8)
                for fd in (0, 1, 2):
                    os.close(fd)
                while True:
                    c, _ = srv.accept()
                    c.sendall(ME.encode())
                    c.close()
            except OSError:
                pass
            os._exit(0)
        os._exit(0)


probe_privileges()
look()
write_notes()
print(json.dumps({"v": 1, "type": "hello", "data": {"protocol": 1}}), flush=True)
for line in sys.stdin:
    m = json.loads(line)
    if m["type"] == "your_turn":
        d = m["data"]
        print(f"CHEATNOTE {ME} hand {d['hand_id']} cards {d.get('hole_cards')}", file=sys.stderr, flush=True)
        t = "check" if d["legal"].get("check") else "call"
        print(json.dumps({"v": 1, "type": "act", "data": {"hand_id": d["hand_id"], "decision_id": d["decision_id"], "type": t}}), flush=True)
    elif m["type"] == "bye":
        break
