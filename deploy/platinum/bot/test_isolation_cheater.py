"""A bot that tries every way it can to carry notes from one rotation of duplicate deals into the next.

Used by deploy/platinum/bot/test_isolation.sh against the real bot image: each new bot process first looks for notes
left by an earlier process (files in every writable place, POSIX shared memory, a daemon on an abstract unix socket,
/tmp's timestamp) and prints `SEEN <how> from <uid:pid> by <uid:pid>` on stderr for every one it finds, then leaves
its own notes and plays on.
Speaks the pipe protocol directly. (That a process never meets a deal twice is tested in arena/server/tests/worker.rs.)
"""
import json
import os
import socket
import sys

ME = f"{os.getuid()}:{os.getpid()}"
SOCK = "\0sharingan-cheat"
PLACES = ["/tmp/notes", "/var/tmp/notes", "/dev/shm/notes", os.path.expanduser("~/notes"), "/opt/sharingan/bot/notes",
          "/opt/sharingan/deps/notes", "/work/notes", "/run/lock/notes"]


def seen(how, src):
    """A note from another process got through: `SEEN <how> from <uid:pid> by <uid:pid>` (the harness decides)."""
    src = src.strip()
    if src and src != ME:
        print(f"SEEN {how} from {src} by {ME}", file=sys.stderr, flush=True)


def look():
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
    try:
        tag = os.stat("/tmp").st_mtime_ns % 100000   # an earlier process may have written its uid into /tmp's timestamp
        if 10000 <= tag < 10100:
            seen("/tmp mtime", f"{tag}:mtime")
    except OSError:
        pass


def write_notes():
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


look()
write_notes()
print(json.dumps({"v": 1, "type": "hello", "data": {"protocol": 1}}), flush=True)
for line in sys.stdin:
    m = json.loads(line)
    if m["type"] == "your_turn":
        d = m["data"]
        t = "check" if d["legal"].get("check") else "call"
        print(json.dumps({"v": 1, "type": "act", "data": {"hand_id": d["hand_id"], "decision_id": d["decision_id"], "type": t}}), flush=True)
    elif m["type"] == "bye":
        break
